use std::{path::Path, thread};

use crossbeam_channel::{bounded, unbounded, Sender};
use futures_channel::oneshot;
use rusqlite::Connection;

use crate::core_notes::error::DBError;

use super::ConnectionBuilder;

pub enum Command {
    Func(Box<dyn FnOnce(&mut Connection) + Send>),
    Shutdown(Box<dyn FnOnce(Result<(), DBError>) + Send>),
}
/// Client represents a single sqlite connection that can be used from async
/// contexts.
#[derive(Clone)]
pub struct AsyncConnection {
    conn_tx: Sender<Command>,
}

impl AsyncConnection {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, DBError> {
        let (open_tx, open_rx) = oneshot::channel();
        println!("Opening Connection");
        Self::open_internal(path, |res| {
            println!("Sending Connection");
            if let Ok(()) = open_tx.send(res) {
                println!("SENT");
            }
            println!("Connection Sent");
        });
        println!("Waiting for Connection");
        let a = open_rx.await?;
        println!("Connection Received");
        a
    }

    // pub fn open_blocking<P: AsRef<Path>>(path: P) -> Result<Self, DBError> {
    //     let (conn_tx, conn_rx) = bounded(1);
    //     Self::open(path, move |res| {
    //         _ = conn_tx.send(res);
    //     });
    //     conn_rx.recv()?
    // }

    fn open_internal<F, P: AsRef<Path>>(path: P, func: F)
    where
        F: FnOnce(Result<Self, DBError>) + Send + 'static,
    {
        let builder = ConnectionBuilder::new(path);
        thread::spawn(move || {
            let (conn_tx, conn_rx) = unbounded();

            let mut conn = match AsyncConnection::create_conn(builder) {
                Ok(conn) => conn,
                Err(err) => {
                    func(Err(err));
                    return;
                }
            };

            let client = Self { conn_tx };
            func(Ok(client));

            while let Ok(cmd) = conn_rx.recv() {
                match cmd {
                    Command::Func(func) => func(&mut conn),
                    Command::Shutdown(func) => match conn.close() {
                        Ok(()) => {
                            func(Ok(()));
                            return;
                        }
                        Err((c, e)) => {
                            conn = c;
                            func(Err(e.into()));
                        }
                    },
                }
            }
        });
    }

    fn create_conn(builder: ConnectionBuilder) -> Result<Connection, DBError> {
        let conn = builder.build()?;

        Ok(conn)
    }

    /// Invokes the provided function with a [`rusqlite::Connection`].
    pub async fn call_immut<F, T>(&self, func: F) -> Result<T, DBError>
    where
        F: FnOnce(&Connection) -> Result<T, DBError> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            _ = tx.send(func(conn));
        })))?;
        rx.await?
    }

    /// Invokes the provided function with a mutable [`rusqlite::Connection`].
    pub async fn call<F, T>(&self, func: F) -> Result<T, DBError>
    where
        F: FnOnce(&mut Connection) -> Result<T, DBError> + Send + 'static,
        T: Send + 'static,
    {
        println!("Creating channels");
        let (tx, rx) = oneshot::channel();
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            println!("ABOUT TO SEND");
            _ = tx.send(func(conn));
            println!("SENT");
        })))?;
        rx.await?
    }

    /// Closes the underlying sqlite connection.
    ///
    /// After this method returns, all calls to `self::conn()` or
    /// `self::conn_mut()` will return an [`DBError::Closed`] error.
    pub async fn close(&self) -> Result<(), DBError> {
        let (tx, rx) = oneshot::channel();
        let func = Box::new(|res| _ = tx.send(res));
        if self.conn_tx.send(Command::Shutdown(func)).is_err() {
            // If the worker thread has already shut down, return Ok here.
            return Ok(());
        }
        // If receiving fails, the connection is already closed.
        rx.await.unwrap_or(Ok(()))
    }

    /// Invokes the provided function with a [`rusqlite::Connection`], blocking
    /// the current thread until completion.
    pub fn conn_blocking<F, T>(&self, func: F) -> Result<T, DBError>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = bounded(1);
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            _ = tx.send(func(conn));
        })))?;
        Ok(rx.recv()??)
    }

    /// Invokes the provided function with a mutable [`rusqlite::Connection`],
    /// blocking the current thread until completion.
    pub fn conn_mut_blocking<F, T>(&self, func: F) -> Result<T, DBError>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = bounded(1);
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            _ = tx.send(func(conn));
        })))?;
        Ok(rx.recv()??)
    }

    /// Closes the underlying sqlite connection, blocking the current thread
    /// until complete.
    ///
    /// After this method returns, all calls to `self::conn_blocking()` or
    /// `self::conn_mut_blocking()` will return an [`DBError::Closed`] error.
    pub fn close_blocking(&self) -> Result<(), DBError> {
        let (tx, rx) = bounded(1);
        let func = Box::new(move |res| _ = tx.send(res));
        if self.conn_tx.send(Command::Shutdown(func)).is_err() {
            return Ok(());
        }
        // If receiving fails, the connection is already closed.
        rx.recv().unwrap_or(Ok(()))
    }
}
