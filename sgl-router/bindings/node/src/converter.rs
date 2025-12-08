use std::sync::Arc;
use std::collections::HashMap;
use uuid::Uuid;

use sglang_router_rs::tokenizer::traits::Tokenizer;
use sglang_router_rs::tokenizer::stream::DecodeStream;
use sglang_router_rs::tool_parser::ToolParser;
use sglang_router_rs::protocols::common::{Tool, ToolChoice, ToolChoiceValue, Usage, StringOrArray};
use sglang_router_rs::tokenizer::stop::StopSequenceDecoder;
use sglang_router_rs::grpc_client::sglang_proto as proto;
use sglang_router_rs::protocols::chat::{ChatCompletionStreamResponse, ChatMessageDelta, ChatStreamChoice};
use sglang_router_rs::grpc_client::sglang_proto::generate_response::Response::{Chunk, Complete, Error};
use sglang_router_rs::routers::grpc::utils::create_stop_decoder;

pub struct ResponseConverter {
    pub tokenizer: Arc<dyn Tokenizer>,
    pub tool_parser: Option<Box<dyn ToolParser>>,
    pub stop_decoder: Option<StopSequenceDecoder>,
    pub model: String,
    pub request_id: String,
    pub created: u64,
    pub system_fingerprint: Option<String>,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
    pub history_tool_calls_count: usize,
    pub stream_buffers: HashMap<u32, String>, // Per-index text buffers
    pub decode_streams: HashMap<u32, DecodeStream>, // Per-index incremental decoders
    pub has_tool_calls: HashMap<u32, bool>, // Track if tool calls were emitted
    pub is_first_chunk: HashMap<u32, bool>, // Track first chunk per index
    pub prompt_tokens: HashMap<u32, i32>, // Track prompt tokens per index (from chunks)
    pub completion_tokens: HashMap<u32, i32>, // Track completion tokens per index (cumulative)
    pub initial_prompt_tokens: Option<i32>, // Initial prompt tokens from request (if available)
    pub skip_special_tokens: bool, // Whether to skip special tokens when decoding
}

impl ResponseConverter {
    pub fn new(
        tokenizer: Arc<dyn Tokenizer>,
        model: String,
        request_id: String,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        stop: Option<StringOrArray>,
        stop_token_ids: Option<Vec<u32>>,
        skip_special_tokens: bool,
    ) -> Self {
        // Create stop decoder if needed
        let stop_decoder = if stop.is_some() || stop_token_ids.is_some() {
            Some(create_stop_decoder(
                &tokenizer,
                stop.as_ref(),
                stop_token_ids.as_ref(),
                skip_special_tokens,
                false, // no_stop_trim
            ))
        } else {
            None
        };

        // Create tool parser if tools are provided
        let tool_parser = if tools.is_some() {
            let factory = sglang_router_rs::tool_parser::ParserFactory::default();
            factory.registry().create_for_model(&model)
        } else {
            None
        };

        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        ResponseConverter {
            tokenizer,
            tool_parser,
            stop_decoder,
            model: model,
            request_id: request_id,
            created,
            system_fingerprint: Some("fp_sglang_node".to_string()),
            tools,
            tool_choice,
            history_tool_calls_count: 0,
            stream_buffers: HashMap::new(),
            decode_streams: HashMap::new(),
            has_tool_calls: HashMap::new(),
            is_first_chunk: HashMap::new(),
            prompt_tokens: HashMap::new(),
            completion_tokens: HashMap::new(),
            initial_prompt_tokens: None,
            skip_special_tokens,
        }
    }

    pub fn convert_chunk(
        &mut self,
        proto_response: proto::GenerateResponse,
    ) -> anyhow::Result<Option<ChatCompletionStreamResponse>> {
        match proto_response.response {
            Some(Chunk(chunk)) => {
                let index = chunk.index;

                // Mark as not first chunk if we've seen this index before
                let is_first = *self.is_first_chunk.entry(index).or_insert(true);
                if is_first {
                    self.is_first_chunk.insert(index, false);
                }

                // Track token counts
                if chunk.prompt_tokens > 0 {
                    self.prompt_tokens.insert(index, chunk.prompt_tokens);
                } else if !self.prompt_tokens.contains_key(&index) {
                     if let Some(initial_prompt) = self.initial_prompt_tokens {
                        self.prompt_tokens.insert(index, initial_prompt);
                    }
                }
                self.completion_tokens.insert(index, chunk.completion_tokens);

                // Process tokens through stop decoder or incremental decoder
                let chunk_text = if let Some(ref mut stop_decoder) = self.stop_decoder {
                    let mut text = String::new();
                    for &token_id in &chunk.token_ids {
                        match stop_decoder.process_token(token_id).unwrap_or(
                            sglang_router_rs::tokenizer::SequenceDecoderOutput::Held
                        ) {
                            sglang_router_rs::tokenizer::SequenceDecoderOutput::Text(t) => text.push_str(&t),
                            sglang_router_rs::tokenizer::SequenceDecoderOutput::StoppedWithText(t) => {
                                text.push_str(&t);
                                break;
                            }
                            sglang_router_rs::tokenizer::SequenceDecoderOutput::Stopped => break,
                            sglang_router_rs::tokenizer::SequenceDecoderOutput::Held => {}
                        }
                    }
                    text
                } else {
                    let decode_stream = self.decode_streams.entry(index).or_insert_with(|| {
                        DecodeStream::new(
                            self.tokenizer.clone(),
                            &[], 
                            self.skip_special_tokens,
                        )
                    });

                    let mut text_parts = Vec::new();
                    for &token_id in &chunk.token_ids {
                        if let Ok(Some(text)) = decode_stream.step(token_id) {
                            text_parts.push(text);
                        }
                    }
                    text_parts.join("")
                };

                if chunk_text.is_empty() && !is_first {
                     return Ok(None);
                }

                // Send first chunk with role
                if is_first {
                    return Ok(Some(ChatCompletionStreamResponse {
                        id: self.request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: self.created,
                        model: self.model.clone(),
                        system_fingerprint: self.system_fingerprint.clone(),
                        choices: vec![ChatStreamChoice {
                            index,
                            delta: ChatMessageDelta {
                                role: Some("assistant".to_string()),
                                content: if chunk_text.is_empty() { None } else { Some(chunk_text.clone()) },
                                tool_calls: None,
                                reasoning_content: None,
                            },
                            logprobs: None,
                            finish_reason: None,
                            matched_stop: None,
                        }],
                        usage: None,
                    }));
                }
                
                if chunk_text.is_empty() {
                    return Ok(None);
                }

                // Update stream buffer
                let stream_buffer = self.stream_buffers.entry(index).or_default();
                stream_buffer.push_str(&chunk_text);

                // Handle tool calls
                if let (Some(ref _tools), Some(ref mut _tool_parser)) = (self.tools.as_ref(), self.tool_parser.as_mut()) {
                    let tool_choice_enabled = !matches!(
                        self.tool_choice,
                        Some(ToolChoice::Value(ToolChoiceValue::None))
                    );

                    if tool_choice_enabled {
                        // TODO: Implement tool parsing
                    }
                }

                Ok(Some(ChatCompletionStreamResponse {
                    id: self.request_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: self.created,
                    model: self.model.clone(),
                    system_fingerprint: self.system_fingerprint.clone(),
                    choices: vec![ChatStreamChoice {
                        index,
                        delta: ChatMessageDelta {
                            role: None,
                            content: Some(chunk_text),
                            tool_calls: None,
                            reasoning_content: None,
                        },
                        logprobs: None,
                        finish_reason: None,
                        matched_stop: None,
                    }],
                    usage: None,
                }))
            }
            Some(Complete(complete)) => {
                let index = complete.index;
                
                // Flush decoder
                let mut final_text = self.stream_buffers.remove(&index).unwrap_or_default();
                if let Some(ref mut decode_stream) = self.decode_streams.get_mut(&index) {
                    if let Ok(Some(remaining)) = decode_stream.flush() {
                        final_text.push_str(&remaining);
                    }
                }
                self.decode_streams.remove(&index);

                // If final_text is empty, it might be a non-streaming request where we need to decode output_ids
                if final_text.is_empty() && !complete.output_ids.is_empty() {
                     match self.tokenizer.decode(&complete.output_ids, self.skip_special_tokens) {
                        Ok(text) => final_text = text,
                        Err(_) => {} // Ignore decoding error
                    }
                }

                // Determine finish reason
                let finish_reason = if complete.finish_reason.is_empty() {
                    "stop".to_string()
                } else {
                    complete.finish_reason.clone()
                };

                 // Build usage
                let mut prompt_tokens = *self.prompt_tokens.get(&index).unwrap_or(&complete.prompt_tokens);
                let mut completion_tokens = *self.completion_tokens.get(&index).unwrap_or(&complete.completion_tokens);
                
                if prompt_tokens == 0 {
                    if let Some(initial) = self.initial_prompt_tokens {
                        prompt_tokens = initial;
                    }
                }
                
                if completion_tokens == 0 && !complete.output_ids.is_empty() {
                    completion_tokens = complete.output_ids.len() as i32;
                }

                let usage = Some(Usage {
                    prompt_tokens: prompt_tokens.max(0) as u32,
                    completion_tokens: completion_tokens.max(0) as u32,
                    total_tokens: (prompt_tokens.max(0) + completion_tokens.max(0)) as u32,
                    completion_tokens_details: None,
                });

                Ok(Some(ChatCompletionStreamResponse {
                    id: self.request_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: self.created,
                    model: self.model.clone(),
                    system_fingerprint: self.system_fingerprint.clone(),
                    choices: vec![ChatStreamChoice {
                        index,
                        delta: ChatMessageDelta {
                            role: None,
                            content: if final_text.is_empty() { None } else { Some(final_text) },
                            tool_calls: None,
                            reasoning_content: None,
                        },
                        logprobs: None,
                        finish_reason: Some(finish_reason),
                        matched_stop: None, // Simplified
                    }],
                    usage,
                }))
            }
            Some(Error(error)) => {
                Err(anyhow::anyhow!("Server error: {} (status: {})", error.message, error.http_status_code))
            }
            None => Ok(None),
        }
    }
}

pub fn generate_tool_call_id(
    model: &str,
    function_name: &str,
    index: usize,
    history_tool_calls_count: usize,
) -> String {
    if model.to_lowercase().contains("kimi") {
        format!("functions.{}:{}", function_name, history_tool_calls_count + index)
    } else {
        format!("call_{}", &Uuid::new_v4().simple().to_string()[..24])
    }
}
