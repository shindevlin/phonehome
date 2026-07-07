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

    /// Send text in chunks to chat_id (optionally into a topic thread).
    /// Returns message_id of the first chunk.
    pub async fn send(&self, chat_id: i64, text: &str) -> Result<i64> {
        self.send_to(chat_id, None, text).await
    }

    pub async fn send_to(&self, chat_id: i64, thread_id: Option<i64>, text: &str) -> Result<i64> {
        let raw = text.trim();
        let clean: &str = if raw.is_empty() { "(no output)" } else { raw };

        let cap = {
            let mut end = clean.len().min(20000);
            while !clean.is_char_boundary(end) { end -= 1; }
            end
        };
        let clean = &clean[..cap];

        let mut first_id: i64 = 0;
        let mut pos = 0;
        while pos < clean.len() {
            let mut end = (pos + CHUNK).min(clean.len());
            while !clean.is_char_boundary(end) { end -= 1; }
            if end == pos { break; }
            let chunk = &clean[pos..end];
            pos = end;

            let mut body = json!({ "chat_id": chat_id, "text": chunk, "parse_mode": "Markdown" });
            if let Some(tid) = thread_id {
                body["message_thread_id"] = json!(tid);
            }
            let resp: serde_json::Value = self.http.post(self.api("sendMessage"))
                .json(&body).send().await?.json().await
                .unwrap_or(serde_json::Value::Null);
            let msg_id = resp["result"]["message_id"].as_i64().unwrap_or(0);
            let msg_id = if msg_id == 0 {
                let mut fb = json!({ "chat_id": chat_id, "text": chunk });
                if let Some(tid) = thread_id { fb["message_thread_id"] = json!(tid); }
                let r: serde_json::Value = self.http.post(self.api("sendMessage"))
                    .json(&fb).send().await?.json().await?;
                r["result"]["message_id"].as_i64().unwrap_or(0)
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
        let arr = resp["result"].as_array().cloned().unwrap_or_default();
        if !arr.is_empty() {
            eprintln!("[tg] {} raw update(s)", arr.len());
        }
        let updates = arr.iter()
            .filter_map(|u| {
                match serde_json::from_value::<Update>(u.clone()) {
                    Ok(v) => Some(v),
                    Err(e) => { eprintln!("[tg] parse err: {e}\n  raw: {u}"); None }
                }
            })
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
    pub message_thread_id: Option<i64>,
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
