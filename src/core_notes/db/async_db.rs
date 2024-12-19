//!
//! Taken from https://github.com/programatik29/tokio-rusqlite
//! Brilliant design, taking it from there to customize the code for my own use case
//!
#![forbid(unsafe_code)]

use std::{path::Path, thread};

// use crossbeam::channel::{Receiver, Sender};
use crossbeam_channel::{Receiver, Sender};
use tokio::sync::oneshot::{self};

use crate::core_notes::error::DBError;

const DB_FILE: &str = "note.sqlite";
const BUG_TEXT: &str = "bug in tokio-rusqlite, please report";

/// The result returned on method calls in this crate.
pub type Result<T> = std::result::Result<T, DBError>;
/// The function called executing against the SQLite connection
type CallFn = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

enum Message {
    Execute(CallFn),
    Close(oneshot::Sender<std::result::Result<(), rusqlite::Error>>),
}

/// A handle to call functions in background thread.
#[derive(Clone)]
pub struct AsyncConnection {
    sender: Sender<Message>,
}

impl AsyncConnection {
    pub async fn open<P: AsRef<Path>>(workspace_path: P) -> Result<Self> {
        let db_path = workspace_path.as_ref().join(DB_FILE);
        let connection = rusqlite::Connection::open(db_path)?;
        let _c = connection.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER,
            true,
        )?;
        start(move || Ok(connection))
            .await
            .map_err(DBError::DBError)
    }

    /// Open a new connection to an in-memory SQLite database.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite open call fails.
    pub async fn open_in_memory() -> Result<Self> {
        start(rusqlite::Connection::open_in_memory)
            .await
            .map_err(DBError::DBError)
    }

    /// Call a function in background thread and get the result
    /// asynchronously.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the database connection has been closed.
    pub async fn call<F, R>(&self, function: F) -> Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R> + 'static + Send,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel::<Result<R>>();

        self.sender
            .send(Message::Execute(Box::new(move |conn| {
                let value = function(conn);
                println!("> Executed function");
                let _ = sender.send(value);
                println!("> Sent");
            })))
            .map_err(|_| DBError::DBConnectionClosed)?;

        println!("< Waiting for value");
        let value = receiver.await.map_err(|_| DBError::DBConnectionClosed)?;
        println!("< Received value");
        value
    }

    /// Call a function in background thread and get the result
    /// asynchronously.
    ///
    /// This method can cause a `panic` if the underlying database connection is closed.
    /// it is a more user-friendly alternative to the [`Connection::call`] method.
    /// It should be safe if the connection is never explicitly closed (using the [`Connection::close`] call).
    ///
    /// Calling this on a closed connection will cause a `panic`.
    pub async fn call_unwrap<F, R>(&self, function: F) -> R
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel::<R>();

        self.sender
            .send(Message::Execute(Box::new(move |conn| {
                let value = function(conn);
                let _ = sender.send(value);
            })))
            .expect("database connection should be open");

        receiver.await.expect(BUG_TEXT)
    }

    pub async fn execute<F>(&self, function: F) -> Result<()>
    where
        F: FnOnce(&rusqlite::Transaction) -> Result<()> + Send + 'static,
    {
        self.call(|conn| {
            let tx = conn.transaction()?;
            function(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Close the database connection.
    ///
    /// This is functionally equivalent to the `Drop` implementation for
    /// `Connection`. It consumes the `Connection`, but on error returns it
    /// to the caller for retry purposes.
    ///
    /// If successful, any following `close` operations performed
    /// on `Connection` copies will succeed immediately.
    ///
    /// On the other hand, any calls to [`Connection::call`] will return a [`Error::ConnectionClosed`],
    /// and any calls to [`Connection::call_unwrap`] will cause a `panic`.
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite close call fails.
    pub async fn close(self) -> Result<()> {
        let (sender, receiver) = oneshot::channel::<std::result::Result<(), rusqlite::Error>>();

        if let Err(crossbeam_channel::SendError(_)) = self.sender.send(Message::Close(sender)) {
            // If the channel is closed on the other side, it means the connection closed successfully
            // This is a safeguard against calling close on a `Copy` of the connection
            return Ok(());
        }

        let result = receiver.await;

        if result.is_err() {
            // If we get a RecvError at this point, it also means the channel closed in the meantime
            // we can assume the connection is closed
            return Ok(());
        }

        result.unwrap().map_err(DBError::DBError)
    }
}

async fn start<F>(open: F) -> rusqlite::Result<AsyncConnection>
where
    F: FnOnce() -> rusqlite::Result<rusqlite::Connection> + Send + 'static,
{
    let (sender, receiver) = crossbeam_channel::unbounded::<Message>();
    let (result_sender, result_receiver) = oneshot::channel();

    println!("Started thread");
    thread::spawn(move || {
        let conn = match open() {
            Ok(c) => c,
            Err(e) => {
                let _ = result_sender.send(Err(e));
                return;
            }
        };

        if let Err(_e) = result_sender.send(Ok(())) {
            return;
        }

        event_loop(conn, receiver);
    });
    println!("Running thread");

    result_receiver
        .await
        .expect(BUG_TEXT)
        .map(|_| AsyncConnection { sender })
}

fn event_loop(mut conn: rusqlite::Connection, receiver: Receiver<Message>) {
    while let Ok(message) = receiver.recv() {
        println!("Message Received");
        match message {
            Message::Execute(f) => f(&mut conn),
            Message::Close(s) => {
                let result = conn.close();

                match result {
                    Ok(v) => {
                        s.send(Ok(v)).expect(BUG_TEXT);
                        break;
                    }
                    Err((c, e)) => {
                        conn = c;
                        s.send(Err(e)).expect(BUG_TEXT);
                    }
                }
            }
        }
    }
    println!("We are done here");
}
