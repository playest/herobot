use std::env;

use chrono::prelude::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs::File;
use std::io::{prelude::*, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::{Receiver, channel}};
use std::thread::{self};
use std::{time::Duration};
use telegram_bot::*;
use tokio::{prelude::*, stream::{StreamExt}};
use thread::sleep;
use futures::{pin_mut, select, join};

struct Bot {
    pub api: Api,
    pub recipient: UserId,
    pub status_indicator: MessageOrChannelPost,
    pub status_indicator_oldness: i8,
    pub in_error: bool,
    watch_dir: PathBuf,
    stream: UpdatesStream,
    rx: Receiver<DebouncedEvent>,
    watcher: RecommendedWatcher,
}

impl Bot {
    fn new(api: Api, recipient: UserId, message: MessageOrChannelPost, watch_dir: PathBuf) -> Bot {
        let api = api;
        let stream = api.stream();
        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
        watcher.watch(&watch_dir, RecursiveMode::Recursive).unwrap();
        Bot {
            api,
            recipient,
            status_indicator: message,
            status_indicator_oldness: 0,
            in_error: false,
            watch_dir,
            stream,
            rx,
            watcher,
        }
    }

    fn error(&self, message: &str, err: Error) {
        //self.in_error = true;
        eprintln!("{} {}", message, err)
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

    async fn maybe_notify(&self, path: &PathBuf) {
        let file = File::open(path.to_owned()).unwrap();
        let mut reader = BufReader::new(file);
        let mut buffer = String::new();
        match reader.read_line(&mut buffer) {
            Ok(_) => {
                if buffer.trim() != "ok" {
                    let file_name = path.file_name().unwrap().to_str().unwrap();
                    let msg = format!("{} not ok: {}", file_name, buffer);
                    self.send(&msg).await;
                    println!("News in {}", file_name);
                }
            },
            Err(err) => eprintln!("Error: {}", err),
        }
    }

    async fn watch_files(&self) {
        println!("Watch files");
        for _ in 1..10 {
            let r = self.rx.try_recv();
            match r {
                Ok(DebouncedEvent::Write(path)) => {
                    self.maybe_notify(&path).await;
                },
                _ => { }
            }
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
                            show_status(self).await;
                        }
                    }
                }
            },
            _ => { }
        }
    }
}

async fn show_status(bot: &Bot) {
    bot.send("Status").await;
}

async fn watch(path: &PathBuf, bot: &mut Bot) -> () {
    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
    watcher.watch(path, RecursiveMode::Recursive).unwrap();

    loop {
        let r = rx.recv();
        match r {
            Ok(event) => {
                match event {
                    DebouncedEvent::Write(path) => {
                        println!("write in {:?}", path);
                        let file = File::open(path.to_owned()).unwrap();
                        let mut reader = BufReader::new(file);
                        let mut buffer = String::new();
                        match reader.read_line(&mut buffer) {
                            Ok(_) => {
                                bot.maybe_notify(&path).await;
                            }
                            Err(err) => eprintln!("Error: {}", err),
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => eprintln!("watch error: {:?}", e),
        };
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

    let api = Api::new(token);

    let id: UserId = UserId::from(recipient_id);
    //println!("id: {}", id);
    //println!("First message");
    let first_message = api.send(id.text("Herobot is back!").disable_notification()).await?;
    let mut bot = Bot::new(api, id, first_message, watch_dir_path);
    //let bot = Arc::new(Mutex::new(bot));
    //let mut bot = Arc::new(RefCell::new(Bot::new(api, id, first_message)));
    //file_watcher.await;
    //let file_watcher = watch(&watch_dir_path, &mut bot);
    //file_watcher.await;


    let mut stream2 = bot.api.stream().timeout(Duration::from_secs(5));

    //let watch_files = bot.watch_files();
    //let watch_commands = bot.watch_commands();
    //let update_status = bot.update_status_indicator();

    loop {
        let watch_files = bot.watch_files();
        //let watch_commands = bot.watch_commands();
        let update_status = bot.update_status_indicator();
        
        //pin_mut!(watch_files, update_status);
        join!(watch_files, update_status);

        println!("Sleep");
        sleep(Duration::from_secs(2));
    }

    Ok(())
}
