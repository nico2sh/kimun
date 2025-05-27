use std::{
    path::{Path, PathBuf},
    thread,
};

use super::{Error, DB_FILE};

use crossbeam_channel::{bounded, unbounded, Sender};
use futures_channel::oneshot;
use rusqlite::{config::DbConfig, Connection, OpenFlags};

enum Command {
    Func(Box<dyn FnOnce(&mut Connection) + Send>),
    Shutdown(Box<dyn FnOnce(Result<(), Error>) + Send>),
}

/// Client represents a single sqlite connection that can be used from async
/// contexts.
#[derive(Clone)]
pub struct Client {
    conn_tx: Sender<Command>,
}

impl Client {
    pub async fn open_async<P: AsRef<Path>>(workspace_path: P) -> Result<Self, Error> {
        let (open_tx, open_rx) = oneshot::channel();
        Self::open(workspace_path, |res| {
            _ = open_tx.send(res);
        });
        open_rx.await?
    }

    pub fn open_blocking<P: AsRef<Path>>(workspace_path: P) -> Result<Self, Error> {
        let (conn_tx, conn_rx) = bounded(1);
        Self::open(workspace_path, move |res| {
            _ = conn_tx.send(res);
        });
        conn_rx.recv()?
    }

    fn open<P: AsRef<Path>, F>(workspace_path: P, func: F)
    where
        F: FnOnce(Result<Self, Error>) + Send + 'static,
    {
        let wp_path = workspace_path.as_ref().to_owned();
        thread::spawn(move || {
            let (conn_tx, conn_rx) = unbounded();

            let mut conn = match Client::create_conn(wp_path) {
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

    fn create_conn<P: AsRef<Path>>(workspace_path: P) -> Result<Connection, Error> {
        // debug!("Opening Database");
        let db_path = workspace_path.as_ref().join(DB_FILE);
        let connection = Connection::open(&db_path)?;
        let _c = connection.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, true)?;
        Ok(connection)
    }

    /// Invokes the provided function with a [`rusqlite::Connection`].
    pub async fn conn<F, T>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            _ = tx.send(func(conn));
        })))?;
        Ok(rx.await??)
    }

    /// Invokes the provided function with a mutable [`rusqlite::Connection`].
    pub async fn conn_mut<F, T>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.conn_tx.send(Command::Func(Box::new(move |conn| {
            _ = tx.send(func(conn));
        })))?;
        Ok(rx.await??)
    }

    /// Invokes the provided function with a [`rusqlite::Connection`].
    ///
    /// Maps the result error type to a custom error; designed to be
    /// used in conjunction with [`query_and_then`](https://docs.rs/rusqlite/latest/rusqlite/struct.CachedStatement.html#method.query_and_then).
    pub async fn conn_and_then<F, T, E>(&self, func: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<rusqlite::Error> + From<Error> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.conn_tx
            .send(Command::Func(Box::new(move |conn| {
                _ = tx.send(func(conn));
            })))
            .map_err(Error::from)?;
        rx.await.map_err(Error::from)?
    }

    /// Invokes the provided function with a mutable [`rusqlite::Connection`].
    ///
    /// Maps the result error type to a custom error; designed to be
    /// used in conjunction with [`query_and_then`](https://docs.rs/rusqlite/latest/rusqlite/struct.CachedStatement.html#method.query_and_then).
    pub async fn conn_mut_and_then<F, T, E>(&self, func: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<rusqlite::Error> + From<Error> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.conn_tx
            .send(Command::Func(Box::new(move |conn| {
                _ = tx.send(func(conn));
            })))
            .map_err(Error::from)?;
        rx.await.map_err(Error::from)?
    }

    /// Closes the underlying sqlite connection.
    ///
    /// After this method returns, all calls to `self::conn()` or
    /// `self::conn_mut()` will return an [`Error::Closed`] error.
    pub async fn close(&self) -> Result<(), Error> {
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
    pub fn conn_blocking<F, T>(&self, func: F) -> Result<T, Error>
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
    pub fn conn_mut_blocking<F, T>(&self, func: F) -> Result<T, Error>
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
    /// `self::conn_mut_blocking()` will return an [`Error::Closed`] error.
    pub fn close_blocking(&self) -> Result<(), Error> {
        let (tx, rx) = bounded(1);
        let func = Box::new(move |res| _ = tx.send(res));
        if self.conn_tx.send(Command::Shutdown(func)).is_err() {
            return Ok(());
        }
        // If receiving fails, the connection is already closed.
        rx.recv().unwrap_or(Ok(()))
    }
}

/// The possible sqlite journal modes.
///
/// For more information, please see the [sqlite docs](https://www.sqlite.org/pragma.html#pragma_journal_mode).
#[derive(Clone, Copy, Debug)]
pub enum JournalMode {
    Delete,
    Truncate,
    Persist,
    Memory,
    Wal,
    Off,
}

impl JournalMode {
    /// Returns the appropriate string representation of the journal mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::Persist => "PERSIST",
            Self::Memory => "MEMORY",
            Self::Wal => "WAL",
            Self::Off => "OFF",
        }
    }
}
