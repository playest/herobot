use std::env;

use std::fs::File;
use futures::StreamExt;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::thread::sleep;
use std::{time::Duration};
use telegram_bot::*;
use std::io::{self, prelude::*, BufReader};

struct Bot {
    pub api: Api,
    pub recipient: UserId,
    pub status_indicator: MessageOrChannelPost,
    pub status_indicator_oldness: i8,
    pub in_error: bool,
}

impl Bot {
    fn new(api: Api, recipient: UserId, message: MessageOrChannelPost) -> Bot {
        Bot {
            api,
            recipient,
            status_indicator: message,
            status_indicator_oldness: 0,
            in_error: false,
        }
    }

    fn error(&mut self, message: &str, err: Error) {
        self.in_error = true;
        eprintln!("{} {}", message, err)
    }

    async fn update_status_indicator(&mut self) {
        if self.status_indicator_oldness < 5 {
            match self
                .api
                .send(
                    self.status_indicator
                        .edit_text(format!("Bot last active pinged at {}", "now")),
                )
                .await
            {
                Ok(_) => self.status_indicator_oldness = 0,
                Err(err) => self.error("Cannot update status indicator", err),
            };
        }
    }

    async fn send(&mut self, text: &str) {
        match self.api.send(self.recipient.text(text)).await {
            Ok(_) => {
                self.status_indicator_oldness += 1;
            }
            Err(err) => {
                self.error("Cannot send message because", err);
            }
        };
    }
}

fn watch(path: PathBuf) -> notify::Result<()> {
    // Create a channel to receive the events.
    let (tx, rx) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path, RecursiveMode::Recursive)?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    loop {
        match rx.recv() {
            Ok(event) => {
                if let DebouncedEvent::Write(path) = event {
                    println!("write in {:?}", path);
                    let file = File::open(path.to_owned()).unwrap();
                    let mut reader = BufReader::new(file);
                    let mut buffer = String::new();
                    match reader.read_line(&mut buffer) {
                        Ok(_) => {
                            println!("first line of {}: {}", path.to_string_lossy(), buffer);
                            if buffer.trim() != "ok" {
                                println!("News in {}", path.to_string_lossy());
                            }
                            else {
                                println!("No news in {}", path.to_string_lossy());
                            }
                        },
                        Err(err) => eprintln!("Error: {}", err),
                    }
                    println!("end!");
                }
            },
            Err(e) => eprintln!("watch error: {:?}", e),
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

    let api = Api::new(token);

    let id: UserId = UserId::from(recipient_id);
    println!("id: {}", id);
    println!("First message");
    api.send(id.text("message to you").disable_notification())
        .await?;

    watch(watch_dir_path);

    Ok(())
}
