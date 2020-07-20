use chrono::prelude::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::{
    mpsc::{channel, Receiver},
    Arc,
};
use std::thread;
use std::{collections::HashMap, io::BufReader, process, time::Duration};
use telegram_bot::*;
use tokio::{stream::StreamExt, sync::Mutex};
use std::fmt::Write;

struct Sender {
    pub api: Api,
    pub recipient: UserId,
    pub in_error: bool,
}

impl Sender {
    fn new(api: Api, recipient: UserId) -> Self {
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
            Ok(_) => { /*self.status_indicator_oldness = 0*/ }
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
            status_indicator: root_message,
        }
    }

    async fn update_status_indicator(&self, sender: &Sender) {
        let text = format!("Herobot pinged at {}", Local::now().trunc_subsecs(0));
        sender
            .update_message(&self.status_indicator, text.as_str())
            .await;
    }
}

struct FileWatcher {
    rx: Receiver<DebouncedEvent>,
    _watcher: RecommendedWatcher, // need _ to store watcher to avoid droping it
}

impl FileWatcher {
    fn new(watch_dir: PathBuf) -> FileWatcher {
        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
        watcher.watch(&watch_dir, RecursiveMode::Recursive).unwrap();
        FileWatcher { rx, _watcher: watcher }
    }

    fn wait_for_change(&self) -> PathBuf {
        println!("Watch files");

        loop {
            let r = self.rx.recv();
            println!("r: {:?}", r);
            match r {
                Ok(DebouncedEvent::Write(path)) | Ok(DebouncedEvent::Create(path)) => {
                    return path;
                }
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
    Stop(Message),
}

struct CommandWatcher<'a> {
    stream: &'a mut UpdatesStream,
}

impl<'a> CommandWatcher<'a> {
    fn new(stream: &'a mut UpdatesStream) -> CommandWatcher<'a> {
        CommandWatcher { stream }
    }

    async fn watch_commands(&mut self) -> Command {
        println!("Watch commands");
        loop {
            if let Some(Ok(update)) = self.stream.next().await {
                // next
                //Ok(Some(update)) => { // try_next
                if let UpdateKind::Message(message) = update.kind {
                    //let now = Utc::now().timestamp();
                    //eprintln!("msg: {}, current: {}, diff: {}", message.date, now, now - message.date);
                    //if message.date + 2 < now {
                    if let MessageKind::Text { ref data, .. } = message.kind {
                        println!("Command {}", data);
                        if data == "/status" {
                            return Command::Status;
                        } else if data == "/stop" {
                            return Command::Stop(message);
                        }
                    }
                    //}
                }
            }
        }
    }
}

fn analyze_file(path: &PathBuf) -> Option<String> {
    let file = File::open(path.to_owned()).unwrap();
    let mut reader = BufReader::new(file);
    let mut buffer = String::new();
    match reader.read_line(&mut buffer) {
        Ok(_) => {
            let file_name = path.file_name().unwrap().to_str().unwrap();
            let msg = format!("{}: {}", file_name, buffer);
            return Some(msg);
        }
        Err(err) => eprintln!("Error: {}", err),
    }
    None
}

struct Status {
    file_path: PathBuf,
    item_name: String,
    last_update: Option<DateTime<Utc>>,
    text: String,
}

impl Status {
    fn new(file_path: PathBuf) -> Self {
        let item_name = Self::extract_file_name(&file_path);
        let last_update = Self::extract_modified_date(&file_path);
        let text = Self::compute_status(&file_path);
        Status {
            file_path,
            item_name,
            last_update,
            text,
        }
    }

    fn update(&mut self) -> bool {
        self.last_update = Self::extract_modified_date(&self.file_path);
        let new_text = Self::compute_status(&self.file_path);
        if new_text != self.text {
            self.text = new_text;
            true
        } else {
            false
        }
    }

    fn compute_status(file_path: &PathBuf) -> String {
        match analyze_file(file_path) {
            Some(text) => text.trim().to_string(),
            None => String::new(),
        }
    }

    fn extract_modified_date(file_path: &PathBuf) -> Option<DateTime<Utc>> {
        file_path.metadata().map_or(None, |md| match md.accessed() {
            Ok(date) => {
                let datetime: DateTime<Utc> = date.into();
                Some(datetime.trunc_subsecs(0))
            }
            Err(_) => None,
        })
    }

    fn extract_file_name(file_path: &PathBuf) -> String {
        file_path
            .file_name()
            .map_or_else(String::new, |f| f.to_string_lossy().to_string())
    }
}
// Store all the status of the bot
struct StatusStore {
    store: HashMap<PathBuf, Status>,
}

impl StatusStore {
    fn new() -> Self {
        StatusStore {
            store: HashMap::new(),
        }
    }

    fn analyze_file(&mut self, file_path: &PathBuf) -> (&Status, bool) {
        if !self.store.contains_key(file_path) {
            let status = Status::new(file_path.clone());
            self.store.insert(file_path.clone(), status);
            (&self.store.get(file_path).unwrap(), true) //? Could not find a way to just return status
        } else {
            let status = self.store.get_mut(file_path).unwrap();
            let changed = status.update();
            (status, changed)
        }
    }

    fn analyze_dir(&mut self, dir_path: &PathBuf) {
        let dir = fs::read_dir(dir_path).unwrap();
        for rfile in dir {
            if let Ok(file) = rfile {
                    let file_path = file.path();
                    if file_path.is_file() {
                        self.analyze_file(&file_path);
                }
            }
        }
    }

    fn summary(&self, dir_path: &PathBuf) -> String {
        let mut global_status_message = String::new();
        let dir = fs::read_dir(dir_path).unwrap();
        for rfile in dir {
            if let Ok(file) = rfile {
                let file_path = file.path();
                if file_path.is_file() {
                    if let Some(status) = self.store.get(&file_path) {
                        //global_status_message.push_str(format!("{} ok at {}{}", status.item_name, status.last_update.map_or_else(|| String::from("unknown date"), |d| d.to_string()), LINE_ENDING).as_str());
                        let text = format!(
                            "{} at {}",
                            status.text.as_str(),
                            status.last_update.map_or_else(
                                || String::from("unknown date"),
                                |d| d.to_string()
                            )
                        );
                        write!(global_status_message, "{} at{}", &status.text, status.last_update.map_or_else(|| "unknown date".into(), |d| d.to_string())).unwrap();
                        global_status_message.push_str(text.as_str());
                    }
                }
            }
        }
        global_status_message
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
    let watch_dir_path2 = watch_dir_path.clone();

    let mut status_store = StatusStore::new();
    status_store.analyze_dir(&watch_dir_path);
    let first_message_string = status_store.summary(&watch_dir_path);

    let status_store_shared = Arc::new(Mutex::new(status_store));
    let api = Api::new(&token);
    let id: UserId = UserId::from(recipient_id);
    if !first_message_string.is_empty() {
        api.send(id.text(first_message_string).disable_notification())
            .await?;
    }

    let first_message = api
        .send(id.text("Herobot is back!").disable_notification())
        .await?;
    let sender = Arc::new(Mutex::new(Sender::new(api, id)));

    // "thread" that updates a message to show that the bot is still online
    let sender_for_thread_status = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let status_updater = StatusUpdater::new(first_message);
        loop {
            thread::sleep(Duration::from_secs(2));
            let sender = tokio_rt.block_on(sender_for_thread_status.lock());
            let task = status_updater.update_status_indicator(&sender);
            tokio_rt.block_on(task);
        }
    });

    // "thread" that send a notification when a file is updated
    let status_store_for_thread_changes = Arc::clone(&status_store_shared);
    let sender_for_thread_changes = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let file_watcher = FileWatcher::new(watch_dir_path);
        loop {
            println!("Changes");
            let changes = file_watcher.wait_for_change();
            let mut status_store = tokio_rt.block_on(status_store_for_thread_changes.lock());
            let (status, changed) = status_store.analyze_file(&changes);
            if changed {
                let sender = tokio_rt.block_on(sender_for_thread_changes.lock());
                let task = sender.send(&status.text);
                tokio_rt.block_on(task);
                println!("changes: {:?}", changes);
            }
        }
    });

    // "thread" that react when a command ("/something") is received in the chat
    let status_store_for_commands_watch = Arc::clone(&status_store_shared);
    let sender_for_commands_watch = Arc::clone(&sender);
    tokio::task::spawn_blocking(move || {
        let mut tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let mut stream = Api::new(&token).stream();
        let mut command_watcher = CommandWatcher::new(&mut stream);
        let mut n = 0;
        loop {
            let command_task = command_watcher.watch_commands();
            match tokio_rt.block_on(command_task) {
                Command::Status => {
                    let status_store = tokio_rt.block_on(status_store_for_commands_watch.lock());
                    let summary = status_store.summary(&watch_dir_path2);
                    let sender = tokio_rt.block_on(sender_for_commands_watch.lock());
                    let task = sender.send(summary.as_str());
                    tokio_rt.block_on(task);
                }
                Command::Stop(m) => {
                    if n == 0 {
                        eprintln!("Ignore first command if /stop");
                        let sender = tokio_rt.block_on(sender_for_commands_watch.lock());
                        tokio_rt
                            .block_on(sender.api.send(m.text_reply("Ignore /stop")))
                            .unwrap();
                    } else {
                        println!("Exiting ... {}", n);
                        let sender = tokio_rt.block_on(sender_for_commands_watch.lock());
                        tokio_rt
                            .block_on(sender.api.send(m.text_reply("Stopping.")))
                            .unwrap();
                        process::exit(0);
                    }
                }
            }
            n += 1;
        }
    });

    Ok(())
}
