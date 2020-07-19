use std::env;

use chrono::prelude::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::{Receiver, channel}};
use std::thread::{self};
use std::{time::Duration, pin::Pin, task::{Poll, Context}};
use telegram_bot::*;
use tokio::{stream::{StreamExt}};

use futures::{Future};

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

struct FileWatcher<'a> {
    sender: &'a Sender,
    rx: Receiver<DebouncedEvent>,
    #[allow(dead_code)] // need this to avoid watcher beeing droped
    watcher: RecommendedWatcher,
}

impl<'a> FileWatcher<'a> {
    fn new(sender: &'a Sender, watch_dir: PathBuf) -> FileWatcher<'a> {
        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
        watcher.watch(&watch_dir, RecursiveMode::Recursive).unwrap();
        FileWatcher {
            sender,
            rx,
            watcher
        }
    }

    async fn maybe_notify(&self, path: PathBuf) {
        let file = File::open(path.to_owned()).unwrap();
        let mut reader = BufReader::new(file);
        let mut buffer = String::new();
        match reader.read_line(&mut buffer) {
            Ok(_) => {
                if buffer.trim() != "ok" {
                    let file_name = path.file_name().unwrap().to_str().unwrap();
                    let msg = format!("{} not ok: {}", file_name, buffer);
                    self.sender.send(&msg).await;
                    println!("News in {}", file_name);
                }
            },
            Err(err) => eprintln!("Error: {}", err),
        }
    }

    async fn watch_files(&self) {
        println!("Watch files");
        let r = self.rx.recv();
        match r {
            Ok(DebouncedEvent::Write(path)) => {
                self.maybe_notify(path).await;
            },
            _ => { }
        }
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

struct DoNothing;

impl Future for DoNothing {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _ctx: &mut Context) -> Poll<Self::Output> {
        Poll::Ready(())
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
    let sender = Sender::new(api_sender, id);
    
    //let file_watcher = FileWatcher::new(&sender, &watch_dir_path);
    //let mut command_watcher = CommandWatcher::new(&sender, &mut stream);

    let thread_file_watcher = thread::spawn(move || {
        let file_watcher = FileWatcher::new(&sender, watch_dir_path);
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            let watch_files = file_watcher.watch_files();
            rt.block_on(watch_files);
        }
    });

    let token2 = token.clone();
    let thread_status_updater = thread::spawn(move || {
        let api = Api::new(&token);
        let status_updater = StatusUpdater::new(&api, first_message);
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            let update_status = status_updater.update_status_indicator();
            rt.block_on(update_status);
            thread::sleep(Duration::from_secs(11));
        }
    });

    /**/
    let thread_command_watcher = thread::spawn(move || {
        let api = Api::new(&token2);
        let mut stream = api.stream();
        let sender = Sender::new(api, id);
        let mut command_watcher = CommandWatcher::new(&sender, &mut stream);
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            
            let watch_commands = command_watcher.watch_commands();
            //watch_commands.await;
            rt.block_on(watch_commands);
        }
    });
    thread_command_watcher.join().unwrap();
    /**/

    thread_file_watcher.join().unwrap();
    thread_status_updater.join().unwrap();

    Ok(())
}
