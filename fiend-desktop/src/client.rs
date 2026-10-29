use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fiend_ipc::{Client, ClientError, ErrorCode, Event, Reply, Request};
use futures::channel::{mpsc, oneshot};
use tokio::sync::mpsc as tokio_mpsc;

use crate::model::{ServiceState, Update};

type Responder = oneshot::Sender<Result<Reply, String>>;

struct Command {
    request: Request,
    reply: Responder,
}

#[derive(Clone)]
pub struct Backend {
    commands: tokio_mpsc::UnboundedSender<Command>,
}

impl Backend {
    pub fn call(&self, request: Request) -> oneshot::Receiver<Result<Reply, String>> {
        let (reply, receiver) = oneshot::channel();
        if let Err(error) = self.commands.send(Command { request, reply }) {
            let _ = error
                .0
                .reply
                .send(Err("The connection to the service is closed".into()));
        }
        receiver
    }

    pub fn offline() -> Self {
        let (commands, mut receiver) = tokio_mpsc::unbounded_channel::<Command>();
        std::thread::spawn(move || {
            while let Some(command) = receiver.blocking_recv() {
                let _ = command.reply.send(Err("The service is not running".into()));
            }
        });
        Self { commands }
    }
}

pub fn spawn(socket: PathBuf) -> (Backend, mpsc::UnboundedReceiver<Update>) {
    let (commands, receiver) = tokio_mpsc::unbounded_channel();
    let (updates, update_receiver) = mpsc::unbounded();
    std::thread::Builder::new()
        .name("fiend-backend".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(worker(socket, receiver, updates));
        })
        .expect("backend thread");
    (Backend { commands }, update_receiver)
}

fn describe(error: &ClientError) -> ServiceState {
    match error {
        ClientError::Daemon(error) if error.code == ErrorCode::Unauthorized => {
            ServiceState::Refused(error.message.clone())
        }
        ClientError::UnsupportedVersion { .. } => ServiceState::Incompatible(error.to_string()),
        ClientError::Daemon(error) if error.code == ErrorCode::UnsupportedVersion => {
            ServiceState::Incompatible(error.message.clone())
        }
        _ if crate::service::is_installed() => ServiceState::NotRunning,
        _ => ServiceState::NotInstalled,
    }
}

async fn worker(
    socket: PathBuf,
    mut commands: tokio_mpsc::UnboundedReceiver<Command>,
    updates: mpsc::UnboundedSender<Update>,
) {
    loop {
        match Client::connect(&socket, "fiend-desktop").await {
            Ok(client) => {
                let client = Arc::new(client);
                if session(&client, &mut commands, &updates).await.is_err() {
                    return;
                }
                if updates
                    .unbounded_send(Update::Service(describe(&ClientError::Closed)))
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                if updates
                    .unbounded_send(Update::Service(describe(&error)))
                    .is_err()
                {
                    return;
                }
            }
        }
        let wait = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                command = commands.recv() => match command {
                    Some(command) => {
                        let _ = command.reply.send(Err("The service is not running".into()));
                    }
                    None => return,
                },
            }
        }
    }
}

async fn load(client: &Client, updates: &mpsc::UnboundedSender<Update>) -> Result<(), ClientError> {
    let _ = updates.unbounded_send(Update::Status(client.status().await?));
    let _ = updates.unbounded_send(Update::Profiles(client.list_profiles().await?));
    let _ = updates.unbounded_send(Update::Settings(client.settings().await?));
    let _ = updates.unbounded_send(Update::Logs(client.logs(1000).await?));
    Ok(())
}

async fn session(
    client: &Arc<Client>,
    commands: &mut tokio_mpsc::UnboundedReceiver<Command>,
    updates: &mpsc::UnboundedSender<Update>,
) -> Result<(), ()> {
    let mut events = client.events();
    if client.subscribe().await.is_err() || load(client, updates).await.is_err() {
        return Ok(());
    }
    if updates
        .unbounded_send(Update::Service(ServiceState::Ready))
        .is_err()
    {
        return Err(());
    }
    loop {
        tokio::select! {
            _ = client.closed() => return Ok(()),
            command = commands.recv() => {
                let Some(command) = command else { return Err(()) };
                let client = client.clone();
                tokio::spawn(async move {
                    let result = client.request(command.request).await.map_err(|error| error.to_string());
                    let _ = command.reply.send(result);
                });
            }
            event = events.recv() => {
                let update = match event {
                    Ok(Event::State(status)) => Update::Status(status),
                    Ok(Event::Stats(stats)) => Update::Stats(stats),
                    Ok(Event::Log(entry)) => Update::Log(entry),
                    Ok(Event::Settings(settings)) => Update::Settings(settings),
                    Ok(Event::ProfilesChanged) => match client.list_profiles().await {
                        Ok(profiles) => Update::Profiles(profiles),
                        Err(_) => continue,
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if load(client, updates).await.is_err() {
                            return Ok(());
                        }
                        continue;
                    }
                    Err(_) => return Ok(()),
                };
                if updates.unbounded_send(update).is_err() {
                    return Err(());
                }
            }
        }
    }
}
