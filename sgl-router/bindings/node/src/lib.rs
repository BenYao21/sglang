#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode, ErrorStrategy};
use std::sync::Arc;
use tokio_stream::StreamExt;
use sglang_router_rs::tokenizer::create_tokenizer_from_file;
use sglang_router_rs::tokenizer::traits::Tokenizer;
use sglang_router_rs::grpc_client::sglang_scheduler::SglangSchedulerClient;
use sglang_router_rs::protocols::chat::ChatCompletionRequest;
use sglang_router_rs::routers::grpc::utils::{process_chat_messages, generate_tool_constraints};
use uuid::Uuid;

mod converter;
use converter::ResponseConverter;

#[napi]
pub struct SglangClient {
    client: Arc<SglangSchedulerClient>,
    tokenizer: Arc<dyn Tokenizer>,
}

#[napi]
impl SglangClient {
    #[napi(factory)]
    pub async fn connect(endpoint: String, tokenizer_path: String) -> Result<Self> {
        let tokenizer = create_tokenizer_from_file(&tokenizer_path)
            .map_err(|e| Error::new(Status::InvalidArg, format!("Failed to create tokenizer: {}", e)))?;

        let client = SglangSchedulerClient::connect(&endpoint).await
            .map_err(|e| Error::new(Status::GenericFailure, format!("Failed to connect: {}", e)))?;

        Ok(SglangClient {
            client: Arc::new(client),
            tokenizer,
        })
    }

    #[napi]
    pub fn tokenizer_encode(&self, text: String) -> Result<Vec<u32>> {
        let encoding = self.tokenizer.encode(&text)
            .map_err(|e| Error::new(Status::GenericFailure, format!("Tokenization failed: {}", e)))?;
        Ok(encoding.token_ids().to_vec())
    }

    #[napi]
    pub fn tokenizer_decode(&self, token_ids: Vec<u32>) -> Result<String> {
        let text = self.tokenizer.decode(&token_ids, true)
             .map_err(|e| Error::new(Status::GenericFailure, format!("Decoding failed: {}", e)))?;
        Ok(text)
    }

    /// Non-streaming chat completion
    /// Returns the full JSON response string
    #[napi]
    pub async fn chat_completion(&self, request_json: String) -> Result<String> {
        let chat_request: ChatCompletionRequest = serde_json::from_str(&request_json)
            .map_err(|e| Error::new(Status::InvalidArg, format!("Failed to parse request JSON: {}", e)))?;

        let processed_messages = process_chat_messages(&chat_request, self.tokenizer.as_ref())
            .map_err(|e| Error::new(Status::GenericFailure, format!("Failed to process messages: {}", e)))?;

        let token_ids = self.tokenizer.encode(&processed_messages.text)
            .map_err(|e| Error::new(Status::GenericFailure, format!("Failed to tokenize: {}", e)))?
            .token_ids()
            .to_vec();
        
        let prompt_tokens = token_ids.len() as i32;

        let tool_constraint = if let Some(tools) = chat_request.tools.as_ref() {
            generate_tool_constraints(tools, &chat_request.tool_choice, &chat_request.model)
                .map_err(|e| Error::new(Status::GenericFailure, format!("Failed to generate tool constraints: {}", e)))?
        } else {
            None
        };

        let request_id = format!("chatcmpl-{}", Uuid::new_v4());
        let proto_request = self.client.build_generate_request_from_chat(
            request_id.clone(),
            &chat_request,
            processed_messages.text,
            token_ids,
            processed_messages.multimodal_inputs,
            tool_constraint,
        ).map_err(|e| Error::new(Status::GenericFailure, format!("Failed to build generate request: {}", e)))?;

        let mut stream = self.client.generate(proto_request).await
            .map_err(|e| Error::new(Status::GenericFailure, format!("Failed to send request: {}", e)))?;

        // Initialize converter
        let mut converter = ResponseConverter::new(
            self.tokenizer.clone(),
            chat_request.model.clone(),
            request_id,
            chat_request.tools.clone(),
            chat_request.tool_choice.clone(),
            chat_request.stop.clone(),
            chat_request.stop_token_ids.clone(),
            chat_request.skip_special_tokens,
        );
        converter.initial_prompt_tokens = Some(prompt_tokens);

        let mut full_content = String::new();
        let mut final_usage = None;
        let mut finish_reason = None;

        while let Some(result) = stream.next().await {
             match result {
                 Ok(proto_response) => {
                     match converter.convert_chunk(proto_response) {
                         Ok(Some(openai_chunk)) => {
                             if let Some(choice) = openai_chunk.choices.first() {
                                 if let Some(content) = &choice.delta.content {
                                     full_content.push_str(content);
                                 }
                                 if choice.finish_reason.is_some() {
                                     finish_reason = choice.finish_reason.clone();
                                 }
                             }
                             if openai_chunk.usage.is_some() {
                                 final_usage = openai_chunk.usage;
                             }
                         },
                         Ok(None) => {},
                         Err(e) => return Err(Error::new(Status::GenericFailure, format!("Conversion error: {}", e))),
                     }
                 }
                 Err(e) => return Err(Error::new(Status::GenericFailure, format!("Stream error: {}", e))),
             }
        }

        let response = serde_json::json!({
            "id": converter.request_id,
            "object": "chat.completion",
            "created": converter.created,
            "model": converter.model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": full_content
                },
                "finish_reason": finish_reason.unwrap_or("stop".to_string())
            }],
            "usage": final_usage
        });

        Ok(response.to_string())
    }

    /// Streaming chat completion
    /// Accepts a callback function that receives JSON chunks
    #[napi(ts_args_type = "requestJson: string, callback: (err: null | Error, chunk: string) => void")]
    pub fn chat_completion_stream(&self, request_json: String, callback: ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>) -> Result<()> {
         let chat_request: ChatCompletionRequest = serde_json::from_str(&request_json)
            .map_err(|e| Error::new(Status::InvalidArg, format!("Failed to parse request JSON: {}", e)))?;
        
        let client = self.client.clone();
        let tokenizer = self.tokenizer.clone();

        // We spawn a tokio task to handle the stream asynchronously
        // NAPI's ThreadsafeFunction allows us to call back into JS from this thread
        tokio::spawn(async move {
            let process_result = async {
                let processed_messages = process_chat_messages(&chat_request, tokenizer.as_ref())
                    .map_err(|e| anyhow::anyhow!("Failed to process messages: {}", e))?;

                let token_ids = tokenizer.encode(&processed_messages.text)
                    .map_err(|e| anyhow::anyhow!("Failed to tokenize: {}", e))?
                    .token_ids()
                    .to_vec();
                
                let prompt_tokens = token_ids.len() as i32;

                let tool_constraint = if let Some(tools) = chat_request.tools.as_ref() {
                    generate_tool_constraints(tools, &chat_request.tool_choice, &chat_request.model)
                        .map_err(|e| anyhow::anyhow!("Failed to generate tool constraints: {}", e))?
                } else {
                    None
                };

                let request_id = format!("chatcmpl-{}", Uuid::new_v4());
                let proto_request = client.build_generate_request_from_chat(
                    request_id.clone(),
                    &chat_request,
                    processed_messages.text,
                    token_ids,
                    processed_messages.multimodal_inputs,
                    tool_constraint,
                ).map_err(|e| anyhow::anyhow!("Failed to build generate request: {}", e))?;

                let mut stream = client.generate(proto_request).await
                    .map_err(|e| anyhow::anyhow!("Failed to send request: {}", e))?;

                let mut converter = ResponseConverter::new(
                    tokenizer.clone(),
                    chat_request.model.clone(),
                    request_id,
                    chat_request.tools.clone(),
                    chat_request.tool_choice.clone(),
                    chat_request.stop.clone(),
                    chat_request.stop_token_ids.clone(),
                    chat_request.skip_special_tokens,
                );
                converter.initial_prompt_tokens = Some(prompt_tokens);

                while let Some(result) = stream.next().await {
                    match result {
                        Ok(proto_response) => {
                             match converter.convert_chunk(proto_response) {
                                 Ok(Some(openai_chunk)) => {
                                     let json_str = serde_json::to_string(&openai_chunk)
                                         .map_err(|e| anyhow::anyhow!("JSON serialization error: {}", e))?;
                                     callback.call(Ok(json_str), ThreadsafeFunctionCallMode::Blocking);
                                 },
                                 Ok(None) => {},
                                 Err(e) => return Err(anyhow::anyhow!("Conversion error: {}", e)),
                             }
                        }
                        Err(e) => return Err(anyhow::anyhow!("Stream error: {}", e)),
                    }
                }
                Ok(())
            }.await;

            if let Err(e) = process_result {
                callback.call(Err(Error::new(Status::GenericFailure, e.to_string())), ThreadsafeFunctionCallMode::Blocking);
            }
        });

        Ok(())
    }
}
