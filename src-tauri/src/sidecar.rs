use serde::{Deserialize, Serialize};
use reqwest::Client;
use base64::{Engine as _, engine::general_purpose};
use std::path::Path;
use tokio::fs;

#[derive(Debug, Serialize)]
pub struct EmbedRequest {
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Content {
    pub parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text { text: String },
    InlineData { inline_data: InlineData },
}

#[derive(Debug, Serialize)]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub struct EmbedResponse {
    pub embedding: Embedding,
}

#[derive(Debug, Deserialize)]
pub struct Embedding {
    pub values: Vec<f32>,
}

pub struct Embedder {
    client: Client,
    api_key: String,
}

impl Embedder {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn embed_text(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2-preview:embedContent?key={}",
            self.api_key
        );

        let request = EmbedRequest {
            content: Content {
                parts: vec![Part::Text { text: text.to_string() }],
            },
            task_type: Some("RETRIEVAL_QUERY".to_string()),
        };

        let response = self.client.post(url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("API Error: {}", error_text).into());
        }

        let resp: EmbedResponse = response.json().await?;
        Ok(resp.embedding.values)
    }

    pub async fn embed_image(&self, path: &Path) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let bytes = fs::read(path).await?;
        let base64_data = general_purpose::STANDARD.encode(bytes);
        
        let mime_type = match path.extension().and_then(|s| s.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            _ => "image/jpeg",
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2-preview:embedContent?key={}",
            self.api_key
        );

        let request = EmbedRequest {
            content: Content {
                parts: vec![Part::InlineData {
                    inline_data: InlineData {
                        mime_type: mime_type.to_string(),
                        data: base64_data,
                    },
                }],
            },
            task_type: Some("RETRIEVAL_DOCUMENT".to_string()),
        };

        let response = self.client.post(url)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("API Error: {}", error_text).into());
        }

        let resp: EmbedResponse = response.json().await?;
        Ok(resp.embedding.values)
    }
}
