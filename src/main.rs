use std::env;

use chrono::prelude::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::{Receiver, channel, RecvError}};
use std::thread::{self};
use std::{time::Duration};
use telegram_bot::*;
use tokio::{stream::{StreamExt}};

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
}

struct StatusUpdater<'a> {
    pub api: &'a Api,
    pub status_indicator_oldness: i8,
    pub status_indicator: MessageOrChannelPost,
}

impl<'a> StatusUpdater<'a> {
    fn new(api: &'a Api, root_message: MessageOrChannelPost) -> StatusUpdater {
        StatusUpdater {
            api,
            status_indicator_oldness: 0,
            status_indicator: root_message
        }
    }

    async fn update_status_indicator(&self) {
        println!("Update status");
        if self.status_indicator_oldness < 5 {
            match self
                .api
                .send(self.status_indicator.edit_text(format!(
                    "Bot last active pinged at {}",
                    Local::now().trunc_subsecs(0)
                )))
                .await
            {
                Ok(_) => { /*self.status_indicator_oldness = 0*/ },
                Err(err) => self.error("Cannot update status indicator", err),
            };
        }
    }

    fn error(&self, message: &str, err: Error) {
        //self.in_error = true;
        eprintln!("{} {}", message, err)
    }
}

struct FileWatcher {
    rx: Receiver<DebouncedEvent>,
    #[allow(dead_code)] // need this to avoid watcher beeing droped
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

    fn get_changes(&self) -> Vec<PathBuf> {
        println!("Watch files");
        let mut changed: Vec<PathBuf> = vec![];

        let r = self.rx.recv();
        println!("r: {:?}", r);
        match r {
            Ok(DebouncedEvent::Write(path)) => {
                changed.push(path);
            },
            Ok(_) => { /* ignore event */ }
            Err(e) => {
                eprintln!("An error happened when polling changes in files: {}", e);
            }
        }

        let mut changed: Vec<PathBuf> = vec![];
        for _ in 1..10 {
            let r = self.rx.try_recv();
            println!("r: {:?}", r);
            match r {
                Ok(DebouncedEvent::Write(path)) => {
                    changed.push(path);
                },
                Ok(_) => { /* ignore event */ }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    break;
                },
                Err(e) => {
                    eprintln!("An error happened when polling changes in files: {}", e);
                }
            }
        }

        return changed;
    }
}

struct CommandWatcher<'a> {
    sender: &'a Sender,
    stream: &'a mut UpdatesStream,
}

impl<'a> CommandWatcher<'a> {
    fn new(sender: &'a Sender, stream: &'a mut UpdatesStream) -> CommandWatcher<'a> {
        CommandWatcher {
            sender,
            stream,
        }
    }

    async fn watch_commands(&mut self) {
        println!("Watch commands");
        let update = self.stream.try_next().await;
        println!("update: {:?}", update);
        match update {
            //Some(Ok(Ok(update))) => { // next
            Ok(Some(update)) => { // try_next
                if let UpdateKind::Message(message) = update.kind {
                    if let MessageKind::Text { ref data, .. } = message.kind {
                        if data == "/status" {
                            self.sender.send("Status").await;
                        }
                    }
                }
            },
            _ => { }
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

    let api_sender = Api::new(&token);

    let file_watcher = FileWatcher::new(watch_dir_path);

    loop {
        let changes = file_watcher.get_changes();
        println!("changes: {:?}" , changes);
    }

    Ok(())
}
