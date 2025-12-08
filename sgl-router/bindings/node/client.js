"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.SglangClient = void 0;
const { SglangClient: NativeClient } = require('./index');
class SglangClient {
    constructor(inner) {
        this.inner = inner;
    }
    static async connect(endpoint, tokenizerPath) {
        const inner = await NativeClient.connect(endpoint, tokenizerPath);
        return new SglangClient(inner);
    }
    encode(text) {
        return this.inner.tokenizerEncode(text);
    }
    decode(tokenIds) {
        return this.inner.tokenizerDecode(tokenIds);
    }
    async chatCompletion(request) {
        const jsonStr = JSON.stringify(request);
        if (request.stream) {
            // Return an async iterator for streaming
            const self = this;
            return {
                [Symbol.asyncIterator]: async function* () {
                    let queue = [];
                    let resolveQueue = null;
                    let done = false;
                    let error = null;
                    const callback = (err, chunk) => {
                        if (err) {
                            error = err;
                        }
                        else {
                            try {
                                // Only push valid chunks
                                if (chunk) {
                                    queue.push(JSON.parse(chunk));
                                }
                                else {
                                    // Empty string usually means end of stream in our simplified convention,
                                    // but our Rust side doesn't send empty strings for EOF explicitly via callback.
                                    // Instead, the callback just stops being called.
                                    // Wait, we need a way to signal completion.
                                    // NAPI ThreadsafeFunction doesn't automatically signal "done".
                                    // We might need to send a special sentinel or just rely on the promise returning?
                                    // Actually, `chat_completion_stream` returns void immediately.
                                    // The callback is called repeatedly.
                                    // We need to modify Rust to send a "done" signal or handle it here.
                                    // Current Rust implementation just stops calling.
                                    // This is tricky. 
                                    // Let's implement a simple "event emitter" style wrapper or fix Rust to send null/undefined for done.
                                    // For now, let's assume valid chunks.
                                }
                            }
                            catch (e) {
                                console.error("Error parsing chunk", e);
                            }
                        }
                        if (resolveQueue) {
                            const resolve = resolveQueue;
                            resolveQueue = null;
                            resolve();
                        }
                    };
                    // Trigger the stream
                    // We need to modify the Rust side to notify us when it's done.
                    // Or we can wrap this in a different way.
                    // For this quick iteration, let's just use the non-streaming method if stream is false.
                    // If we want real streaming, we need to handle the end of stream.
                    // Let's stick to non-streaming for now in this wrapper unless we update Rust to send a "Done" signal.
                    throw new Error("Streaming interface requires update to native binding to signal completion.");
                }
            };
        }
        else {
            const responseStr = await this.inner.chatCompletion(jsonStr);
            return JSON.parse(responseStr);
        }
    }
    // Expose a raw stream method that accepts a callback
    chatCompletionStream(request, onChunk, onError) {
        const jsonStr = JSON.stringify(request);
        this.inner.chatCompletionStream(jsonStr, (err, chunkStr) => {
            if (err) {
                onError(err);
            }
            else {
                // Check for null or special end-of-stream signal
                if (chunkStr === "null" || chunkStr === null) {
                    onChunk(null); // Signal end of stream
                    return;
                }
                try {
                    const chunk = JSON.parse(chunkStr);
                    onChunk(chunk);
                }
                catch (e) {
                    onError(e);
                }
            }
        });
    }
}
exports.SglangClient = SglangClient;
