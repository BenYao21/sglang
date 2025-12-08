/// <reference types="node" />
import { SglangClient, ChatCompletionRequest } from './client';
import assert from 'assert';

// Configuration
const CONFIG = {
    endpoint: process.env.SGLANG_ROUTER_ENDPOINT || "grpc://localhost:30000",
    tokenizerPath: process.env.TOKENIZER_PATH || "/home/ben/.cache/huggingface/hub/models--Qwen--Qwen3-0.6B/snapshots/c1899de289a04d12100db370d81485cdf75e47ca",
    model: "Qwen/Qwen3-0.6B",
};

// --- Unit Tests (Mimicking Go's client_test.go) ---

async function testClientConfig() {
    console.log("\n🧪 TestClientConfig");
    try {
        const client = await SglangClient.connect(CONFIG.endpoint, CONFIG.tokenizerPath);
        assert(client instanceof SglangClient, "Should return SglangClient instance");
        console.log("  ✅ Valid config passed");
    } catch (e) {
        console.error("  ❌ Valid config failed", e);
        throw e;
    }
}

function testChatCompletionRequestValidation() {
    console.log("\n🧪 TestChatCompletionRequestValidation");
    const req: ChatCompletionRequest = {
        model: "default",
        messages: [{ role: "user", content: "test" }],
        stream: false
    };
    
    assert.strictEqual(req.model, "default");
    assert.strictEqual(req.messages.length, 1);
    assert.strictEqual(req.messages[0].role, "user");
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

    assert.strictEqual(resp.id, "test-id");
    assert.strictEqual(resp.choices.length, 1);
    assert.strictEqual(resp.choices[0].message.content, "Hello");
    assert.strictEqual(resp.usage.total_tokens, 30);
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

    assert.strictEqual(chunk.id, "stream-id");
    assert(chunk.choices.length > 0);
    assert.strictEqual(chunk.choices[0].delta.content, "Hello");
    console.log("  ✅ Streaming chunk structure valid");
}

// --- Integration Tests (Real calls) ---

async function testIntegration(client: SglangClient) {
    console.log("\n--- Integration Tests ---");

    // Tokenizer
    console.log("\n🧪 TestTokenizer");
    const text = "Hello world";
    const tokens = client.encode(text);
    console.log(`  Encoded: [${tokens}]`);
    assert(Array.isArray(tokens));
    assert(tokens.length > 0);
    const decoded = client.decode(tokens);
    console.log(`  Decoded: ${decoded}`);
    assert.strictEqual(decoded, text);
    console.log("  ✅ Tokenizer roundtrip passed");

    // Chat Completion
    console.log("\n🧪 TestChatCompletion (Real)");
    const req = {
        model: CONFIG.model,
        messages: [{ role: "user", content: "What is 1+1?" }],
        max_tokens: 16,
        temperature: 0
    };
    const resp = await client.chatCompletion(req);
    console.log(`  Response: ${JSON.stringify(resp.choices[0].message.content)}`);
    assert(resp.choices[0].message.content.length > 0);
    assert.strictEqual(typeof resp.usage.total_tokens, 'number');
    console.log("  ✅ Chat completion passed");

    // Streaming
    console.log("\n🧪 TestChatCompletionStream (Real)");
    const streamReq = {
        model: CONFIG.model,
        messages: [{ role: "user", content: "Count to 3" }],
        max_tokens: 32,
        temperature: 0,
        stream: true
    };
    
    let fullContent = "";
    await new Promise<void>((resolve, reject) => {
        client.chatCompletionStream(streamReq, (chunk) => {
            if (chunk.choices && chunk.choices[0].delta.content) {
                fullContent += chunk.choices[0].delta.content;
                process.stdout.write(chunk.choices[0].delta.content);
            }
        }, (err) => {
            console.error("Stream error:", err);
            reject(err);
        });

        // Simple timeout since we don't have explicit done signal yet
        setTimeout(() => {
            console.log("\n  (Stream timeout)");
            if (fullContent.length > 0) resolve();
            else reject(new Error("No content received"));
        }, 3000);
    });
    console.log(`  Full stream content: ${fullContent}`);
    assert(fullContent.length > 0);
    console.log("  ✅ Chat stream passed");
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
        const client = await SglangClient.connect(CONFIG.endpoint, CONFIG.tokenizerPath);
        await testIntegration(client);

        console.log("\n🎉 All tests passed!");
    } catch (err) {
        console.error("\n❌ Test failed:", err);
        process.exit(1);
    }
}

if (require.main === module) {
    main();
}
