use anyhow::Result;
use reqwest::Client;
use serde_json::json;

const CHUNK: usize = 4000;

pub struct TelegramClient {
    token: String,
    http: Client,
}

impl TelegramClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.to_string(),
            http: Client::new(),
        }
    }

    fn api(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    /// Send text in chunks, returns message_id of the first chunk (for deletions)
    pub async fn send(&self, chat_id: i64, text: &str) -> Result<i64> {
        let clean = if text.trim().is_empty() { "(no output)" } else { text };
        let mut first_id: i64 = 0;
        for i in (0..clean.len().min(20000)).step_by(CHUNK) {
            let chunk = &clean[i..(i + CHUNK).min(clean.len())];
            let resp: serde_json::Value = self.http.post(self.api("sendMessage"))
                .json(&json!({ "chat_id": chat_id, "text": chunk, "parse_mode": "Markdown" }))
                .send().await?
                .json().await
                .unwrap_or(serde_json::Value::Null);
            let msg_id = resp["result"]["message_id"].as_i64().unwrap_or(0);
            // fall back to plain if Markdown failed
            let msg_id = if msg_id == 0 {
                let fb: serde_json::Value = self.http.post(self.api("sendMessage"))
                    .json(&json!({ "chat_id": chat_id, "text": chunk }))
                    .send().await?.json().await?;
                fb["result"]["message_id"].as_i64().unwrap_or(0)
            } else { msg_id };
            if first_id == 0 { first_id = msg_id; }
        }
        Ok(first_id)
    }

    pub async fn delete(&self, chat_id: i64, message_id: i64) -> Result<()> {
        self.http.post(self.api("deleteMessage"))
            .json(&json!({ "chat_id": chat_id, "message_id": message_id }))
            .send().await?;
        Ok(())
    }

    pub async fn get_file_path(&self, file_id: &str) -> Result<String> {
        let resp: serde_json::Value = self.http.post(self.api("getFile"))
            .json(&json!({ "file_id": file_id }))
            .send().await?
            .json().await?;
        Ok(resp["result"]["file_path"].as_str().unwrap_or("").to_string())
    }

    pub async fn download_file(&self, file_path: &str) -> Result<Vec<u8>> {
        let url = format!("https://api.telegram.org/file/bot{}/{}", self.token, file_path);
        let bytes = self.http.get(&url).send().await?.bytes().await?;
        Ok(bytes.to_vec())
    }

    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>> {
        let resp: serde_json::Value = self.http.post(self.api("getUpdates"))
            .json(&json!({ "offset": offset, "timeout": 10, "limit": 100 }))
            .send().await?
            .json().await?;
        let updates = resp["result"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|u| serde_json::from_value(u.clone()).ok())
            .collect();
        Ok(updates)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
pub struct Message {
    pub message_id: i64,
    pub from: Option<User>,
    pub chat: Chat,
    pub text: Option<String>,
    pub voice: Option<AudioFile>,
    pub audio: Option<AudioFile>,
}

#[derive(Debug, serde::Deserialize)]
pub struct AudioFile {
    pub file_id: String,
    pub duration: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct User {
    pub id: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct Chat {
    pub id: i64,
}
