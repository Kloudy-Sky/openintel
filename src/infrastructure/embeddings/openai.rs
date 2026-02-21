use crate::domain::error::DomainError;
use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct OpenAiRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiEmbedding>,
}

#[derive(Deserialize)]
struct OpenAiEmbedding {
    embedding: Vec<f32>,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "text-embedding-3-small".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiProvider {
    async fn embed(
        &self,
        texts: &[String],
        _input_type: InputType,
    ) -> Result<Vec<Vec<f32>>, DomainError> {
        let resp = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&OpenAiRequest {
                input: texts.to_vec(),
                model: self.model.clone(),
            })
            .send()
            .await
            .map_err(|e| DomainError::Embedding(format!("OpenAI API error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::Embedding(format!(
                "OpenAI API {status}: {body}"
            )));
        }

        let result: OpenAiResponse = resp
            .json()
            .await
            .map_err(|e| DomainError::Parse(format!("Parse error: {e}")))?;
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize {
        match self.model.as_str() {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        }
    }
}
