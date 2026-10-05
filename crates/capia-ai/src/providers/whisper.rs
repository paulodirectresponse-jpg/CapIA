//! STT local por servidor `whisper.cpp` (`/inference`, loopback). Sem servidor ⇒ erro estruturado
//! `NOT_CONFIGURED` ("não instalado/configurado"); **nunca** cai para nuvem (a escolha de rota é do
//! Router, que respeita a privacidade `local-only`).

use super::{CallCtx, ModelProvider};
use crate::error::{ErrorCode, ProviderError};
use crate::http::HttpClient;
use crate::registry::{ProviderConfig, ProviderKind};
use crate::stt::{SttRequest, Transcript};
use crate::types::{ChatRequest, ChatStream};
use async_trait::async_trait;
use serde_json::Value;

pub struct WhisperProvider {
    id: String,
    http: HttpClient,
}

impl core::fmt::Debug for WhisperProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WhisperProvider")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl WhisperProvider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            id: cfg.id.clone(),
            http: HttpClient::new(cfg)?,
        })
    }

    fn not_installed(e: ProviderError) -> ProviderError {
        if e.code == ErrorCode::ProviderUnavailable && e.status.is_none() {
            ProviderError::new(
                ErrorCode::NotConfigured,
                "local transcription server is not reachable: install/start a whisper.cpp server and set its address in the provider",
            )
        } else {
            e
        }
    }
}

#[async_trait]
impl ModelProvider for WhisperProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::WhisperLocal
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(&self, _req: &ChatRequest, _ctx: &CallCtx) -> Result<ChatStream, ProviderError> {
        Err(ProviderError::unsupported("text_generation"))
    }

    async fn transcribe(
        &self,
        req: &SttRequest,
        ctx: &CallCtx,
    ) -> Result<Transcript, ProviderError> {
        let part = reqwest::multipart::Part::bytes(req.audio.clone())
            .file_name(req.filename.clone())
            .mime_str(&req.mime)
            .map_err(|_| ProviderError::new(ErrorCode::InvalidRequest, "invalid audio mime"))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("response_format", "verbose_json")
            .text("temperature", "0.0");
        if let Some(l) = &req.language {
            form = form.text("language", l.clone());
        }
        let rb = self.http.post("inference")?.multipart(form);
        let v: Value = self
            .http
            .send(rb, &ctx.cancel)
            .await
            .map_err(Self::not_installed)?
            .json(&ctx.cancel)
            .await?;
        super::openai::parse_verbose_json(&v)
    }
}
