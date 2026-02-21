use crate::domain::error::DomainError;
use crate::domain::ports::embedding_port::{EmbeddingProvider, InputType};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct VoyageProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

#[derive(Serialize)]
struct VoyageRequest {
    input: Vec<String>,
    model: String,
    input_type: String,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageEmbedding>,
}

#[derive(Deserialize)]
struct VoyageEmbedding {
    embedding: Vec<f32>,
}

impl VoyageProvider {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "voyage-4-lite".to_string()),
            base_url: base_url.unwrap_or_else(|| "https://api.voyageai.com".to_string()),
        }
    }

    fn model_dimension(model: &str) -> usize {
        match model {
            "voyage-4-lite" => 512,
            "voyage-3-lite" => 512,
            "voyage-3" => 1024,
            "voyage-3-large" | "voyage-large-2" => 1536,
            "voyage-code-3" => 1024,
            _ => 512,
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for VoyageProvider {
    async fn embed(
        &self,
        texts: &[String],
        input_type: InputType,
    ) -> Result<Vec<Vec<f32>>, DomainError> {
        let it = match input_type {
            InputType::Document => "document",
            InputType::Query => "query",
        };

        let url = format!("{}/v1/embeddings", self.base_url);

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&VoyageRequest {
                input: texts.to_vec(),
                model: self.model.clone(),
                input_type: it.to_string(),
            })
            .send()
            .await
            .map_err(|e| DomainError::Embedding(format!("Voyage API error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(DomainError::Embedding(format!(
                "Voyage API {status}: {body}"
            )));
        }

        let result: VoyageResponse = resp
            .json()
            .await
            .map_err(|e| DomainError::Parse(format!("Parse error: {e}")))?;
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimension(&self) -> usize {
        Self::model_dimension(&self.model)
    }
}
