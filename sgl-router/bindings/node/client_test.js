"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
/// <reference types="node" />
const client_1 = require("./client");
const assert_1 = __importDefault(require("assert"));
// Configuration
const CONFIG = {
    endpoint: process.env.SGLANG_ROUTER_ENDPOINT || "grpc://localhost:30000",
    tokenizerPath: process.env.TOKENIZER_PATH || "/root/.cache/huggingface/hub/models--Qwen--Qwen2.5-1.5B/snapshots/8faed761d45a263340a0528343f099c05c9a4323",
    model: "Qwen/Qwen2.5-1.5B",
};
// --- Unit Tests (Mimicking Go's client_test.go) ---
async function testClientConfig() {
    console.log("\n🧪 TestClientConfig");
    try {
        const client = await client_1.SglangClient.connect(CONFIG.endpoint, CONFIG.tokenizerPath);
        (0, assert_1.default)(client instanceof client_1.SglangClient, "Should return SglangClient instance");
        console.log("  ✅ Valid config passed");
    }
    catch (e) {
        console.error("  ❌ Valid config failed", e);
        throw e;
    }
}
function testChatCompletionRequestValidation() {
    console.log("\n🧪 TestChatCompletionRequestValidation");
    const req = {
        model: "default",
        messages: [{ role: "user", content: "test" }],
        stream: false
    };
    assert_1.default.strictEqual(req.model, "default");
    assert_1.default.strictEqual(req.messages.length, 1);
    assert_1.default.strictEqual(req.messages[0].role, "user");
    console.log("  ✅ Request structure valid");
}
function testChatCompletionResponseTypes() {
    console.log("\n🧪 TestChatCompletionResponseTypes");
    const resp = {
        id: "test-id",
        model: "default",
        created: 1234567890,
        choices: [{
                index: 0,
                message: { role: "assistant", content: "Hello" },
                finish_reason: "stop"
            }],
        usage: {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30
        }
    };
    assert_1.default.strictEqual(resp.id, "test-id");
    assert_1.default.strictEqual(resp.choices.length, 1);
    assert_1.default.strictEqual(resp.choices[0].message.content, "Hello");
    assert_1.default.strictEqual(resp.usage.total_tokens, 30);
    console.log("  ✅ Response structure valid");
}
function testStreamingResponseTypes() {
    console.log("\n🧪 TestStreamingResponseTypes");
    const chunk = {
        id: "stream-id",
        created: 1234567890,
        choices: [{
                index: 0,
                delta: { content: "Hello" },
                finish_reason: null
            }]
    };
    assert_1.default.strictEqual(chunk.id, "stream-id");
    (0, assert_1.default)(chunk.choices.length > 0);
    assert_1.default.strictEqual(chunk.choices[0].delta.content, "Hello");
    console.log("  ✅ Streaming chunk structure valid");
}
// --- Integration Tests (Real calls) ---
async function testIntegration(client) {
    console.log("\n--- Integration Tests ---");
    // Tokenizer
    console.log("\n🧪 TestTokenizer");
    const text = "Hello world";
    const tokens = client.encode(text);
    console.log(`  Encoded: [${tokens}]`);
    (0, assert_1.default)(Array.isArray(tokens));
    (0, assert_1.default)(tokens.length > 0);
    const decoded = client.decode(tokens);
    console.log(`  Decoded: ${decoded}`);
    assert_1.default.strictEqual(decoded, text);
    console.log("  ✅ Tokenizer roundtrip passed");
    // Chat Completion
    console.log("\n🧪 TestChatCompletion (Real)");
    const req = {
        model: CONFIG.model,
        messages: [{ role: "user", content: "Write a haiku about coding." }],
        max_tokens: 64,
        temperature: 0.7
    };
    const resp = await client.chatCompletion(req);
    console.log(`  Response: ${JSON.stringify(resp.choices[0].message.content)}`);
    (0, assert_1.default)(resp.choices[0].message.content.length > 0);
    assert_1.default.strictEqual(typeof resp.usage.total_tokens, 'number');
    console.log("  ✅ Chat completion passed");
    // Streaming
    console.log("\n🧪 TestChatCompletionStream (Real)");
    const streamReq = {
        model: CONFIG.model,
        messages: [{ role: "user", content: "Write a few sentences about the ocean." }],
        max_tokens: 128,
        temperature: 0.7,
        stream: true
    };
    let fullContent = "";
    const streamPromise = new Promise((resolve, reject) => {
        client.chatCompletionStream(streamReq, (chunk) => {
            if (chunk === null) {
                // End of stream
                resolve();
                return;
            }
            if (chunk.choices && chunk.choices[0].delta.content) {
                fullContent += chunk.choices[0].delta.content;
                process.stdout.write(chunk.choices[0].delta.content);
            }
        }, (err) => {
            console.error("Stream error:", err);
            reject(err);
        });
    });
    // Add a safety timeout
    const timeoutPromise = new Promise((_, reject) => {
        setTimeout(() => reject(new Error("Stream timed out after 10s")), 10000);
    });
    try {
        await Promise.race([streamPromise, timeoutPromise]);
        console.log(`\n  Full stream content: ${fullContent}`);
        (0, assert_1.default)(fullContent.length > 0);
        console.log("  ✅ Chat stream passed");
    }
    catch (e) {
        console.error("\n  ❌ Chat stream failed:", e);
        throw e;
    }
}
async function main() {
    try {
        // Run Unit Tests
        testChatCompletionRequestValidation();
        testChatCompletionResponseTypes();
        testStreamingResponseTypes();
        // Run Client Config Test (Connects)
        await testClientConfig();
        // Connect for Integration Tests
        const client = await client_1.SglangClient.connect(CONFIG.endpoint, CONFIG.tokenizerPath);
        await testIntegration(client);
        console.log("\n🎉 All tests passed!");
    }
    catch (err) {
        console.error("\n❌ Test failed:", err);
        process.exit(1);
    }
}
if (require.main === module) {
    main();
}
