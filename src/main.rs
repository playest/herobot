use std::env;

use chrono::prelude::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::{Receiver, channel}, Arc};
use std::thread::{self};
use std::{time::Duration};
use telegram_bot::*;
use tokio::{stream::{StreamExt}, sync::Mutex};

struct Sender {
    pub api: Api,
    pub recipient: UserId,
    pub in_error: bool,
}

impl Sender {
    fn new(api: Api, recipient: UserId) -> Self {
        let api = api;

        Sender {
            api,
            recipient,
            in_error: false,
        }
    }

    fn error(&self, message: &str, err: Error) {
        //self.in_error = true;
        eprintln!("{} {}", message, err)
    }

    async fn send(&self, text: &str) {
        match self.api.send(self.recipient.text(text)).await {
            Ok(_) => {
                //self.status_indicator_oldness += 1;
            }
            Err(err) => {
                self.error("Cannot send message because", err);
            }
        };
    }

    async fn update_message(&self, message: &MessageOrChannelPost, text: &str) {
        match self.api.send(message.edit_text(text)).await {
            Ok(_) => { /*self.status_indicator_oldness = 0*/ },
            Err(err) => self.error("Cannot update message", err),
        };
    }
}

struct StatusUpdater {
    pub status_indicator: MessageOrChannelPost,
}

impl StatusUpdater {
    fn new(root_message: MessageOrChannelPost) -> StatusUpdater {
        StatusUpdater {
            status_indicator: root_message
        }
    }

    async fn update_status_indicator(&self, sender: &Sender) {
        println!("Update status");
        let text = format!("Bot last active pinged at {}", Local::now().trunc_subsecs(0));
        sender.update_message(&self.status_indicator, text.as_str()).await;
    }
}

struct FileWatcher {
    rx: Receiver<DebouncedEvent>,
    #[allow(dead_code)] // need this to store watcher to avoid droping it
    watcher: RecommendedWatcher,
}

impl FileWatcher {
    fn new(watch_dir: PathBuf) -> FileWatcher {
        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
        watcher.watch(&watch_dir, RecursiveMode::Recursive).unwrap();
        FileWatcher {
            rx,
            watcher
        }
    }

    fn wait_for_change(&self) -> PathBuf {
        println!("Watch files");

        loop {
            let r = self.rx.recv();
            println!("r: {:?}", r);
            match r {
                Ok(DebouncedEvent::Write(path)) => {
                    return path;
                },
                Ok(_) => { /* ignore event */ }
                Err(e) => {
                    eprintln!("An error happened when polling changes in files: {}", e);
                }
            }
        }
    }
}

enum Command {
    Status,
}

struct CommandWatcher<'a> {
    stream: &'a mut UpdatesStream,
}

impl<'a> CommandWatcher<'a> {
    fn new(stream: &'a mut UpdatesStream) -> CommandWatcher<'a> {
        CommandWatcher {
            stream,
        }
    }

    async fn watch_commands(&mut self) -> Command {
        println!("Watch commands");
        loop {
            match self.stream.next().await {
                Some(Ok(update)) => { // next
                //Ok(Some(update)) => { // try_next
                    if let UpdateKind::Message(message) = update.kind {
                        if let MessageKind::Text { ref data, .. } = message.kind {
                            if data == "/status" {
                                return Command::Status;
                            }
                        }
                    }
                },
                _ => { }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let recipient_id: i64 = env::var("RID")
        .expect("RID not set")
        .parse()
        .expect("RID not an integer");
    let watch_dir_path = env::var("WATCH_DIR").map_or(
        Path::new(&dirs::home_dir().unwrap().to_str().unwrap().to_owned()).join(".herobot"),
        |d| Path::new(&d).to_path_buf(),
    );

    if !watch_dir_path.exists() {
        panic!(format!(
            "{} does not exists",
            watch_dir_path.to_string_lossy()
        ));
    }

    let api = Api::new(&token);
    let id: UserId = UserId::from(recipient_id);
    let first_message = api.send(id.text("Herobot is back!").disable_notification()).await?;
    let sender = Arc::new(Mutex::new(Sender::new(api, id)));   

    let sender_for_thread_status = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let status_updater = StatusUpdater::new(first_message);
        loop {
            println!("Update status!");
            thread::sleep(Duration::from_secs(2));
            let sender = sender_for_thread_status.lock();
            let sender = tokio_rt.block_on(sender);
            let task = status_updater.update_status_indicator(&sender);
            tokio_rt.block_on(task);
        }
    });

    let sender_for_thread_changes = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let file_watcher = FileWatcher::new(watch_dir_path);
        loop {
            println!("Changes");
            let changes = file_watcher.wait_for_change();
            let sender = sender_for_thread_changes.lock();
            let sender = tokio_rt.block_on(sender);
            let text = format!("Changes in: {}", changes.to_string_lossy());
            let task = sender.send(&text);
            tokio_rt.block_on(task);
            println!("changes: {:?}" , changes);
        }
    });

    let sender_for_commands_watch = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let api = Api::new(&token);
        let mut stream = api.stream();
        let mut command_watcher = CommandWatcher::new(&mut stream);
        loop {
            let command_task = command_watcher.watch_commands();
            match tokio_rt.block_on(command_task) {
                Command::Status => {
                    let sender = sender_for_commands_watch.lock();
                    let sender = tokio_rt.block_on(sender);
                    let task = sender.send("The status here!");
                    tokio_rt.block_on(task);
                },
            }
        }
    });

    Ok(())
}
