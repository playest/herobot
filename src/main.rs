use std::{env};

use futures::StreamExt;
use telegram_bot::*;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

struct StatusIndicator {
    pub message: MessageOrChannelPost
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let token = env::var("TELEGRAM_BOT_TOKEN").expect("TELEGRAM_BOT_TOKEN not set");
    let recipient_id: i64 = env::var("RID").expect("RID not set").parse().expect("RID not an integer");
    let watch_dir_path = env::var("WATCH_DIR").map_or(
        Path::new(&dirs::home_dir().unwrap().to_str().unwrap().to_owned()).join(".herobot"),
        |d| Path::new(&d).to_path_buf()
    );
    
    if !watch_dir_path.exists() {
        panic!(format!("{} does not exists", watch_dir_path.to_string_lossy()));
    }

    let api = Api::new(token);

    let id: UserId = UserId::from(recipient_id);
    println!("id: {}", id);
    println!("First message");
    let r = api.send(id.text("message to you").disable_notification()).await?;
    
    println!("Edit 1");
    sleep(Duration::from_secs(5));
    api.send(r.edit_text("edited")).await?;

    println!("In between");
    api.send(id.text("in between")).await?;

    println!("Edit 2");
    sleep(Duration::from_secs(5));
    api.send(r.edit_text("edited2")).await?;

    // Fetch new updates via long poll method
    let mut stream = api.stream();

    while let Some(update) = stream.next().await {
        // If the received update contains a new message...
        let update = update?;
        if let UpdateKind::Message(message) = update.kind {
            if let MessageKind::Text { ref data, .. } = message.kind {
                // Print received text message to stdout.
                println!("<{}>: {}", &message.from.first_name, data);

                // Answer message with "Hi".
                api.send(
                    message.text_reply(
                        format!("Hi, {}! You just wrote '{}'", &message.from.first_name, data)
                    )
                ).await?;
                
                //api.send(message.chat.text("hello")).await?;
            }
        }
    }
    Ok(())
}
