#![deny(clippy::all)]

use napi_derive::napi;
use sglang_router::{config, server, PolicyType as RsPolicyType};
use std::collections::HashMap;

// 1. expose enum
#[napi]
pub enum PolicyType {
    Random,
    RoundRobin,
    CacheAware,
    PowerOfTwo,
}

// mapping helper function
impl From<PolicyType> for RsPolicyType {
    fn from(val: PolicyType) -> Self {
        match val {
            PolicyType::Random => RsPolicyType::Random,
            PolicyType::RoundRobin => RsPolicyType::RoundRobin,
            PolicyType::CacheAware => RsPolicyType::CacheAware,
            PolicyType::PowerOfTwo => RsPolicyType::PowerOfTwo,
        }
    }
}

// 2. define config interface (NAPI will automatically generate TS interface)
#[napi(object)]
pub struct RouterOptions {
    pub host: String,
    pub port: u16,
    pub workerUrls: Vec<String>,
    pub policy: Option<PolicyType>,
    pub maxConcurrentRequests: Option<i32>,
    // todo: other options, refer to bindings/python/src/lib.rs
}

// 3. Router class
#[napi]
pub struct Router {
    inner_config: config::RouterConfig,
}

#[napi]
impl Router {
    #[napi(constructor)]
    pub fn new(options: RouterOptions) -> napi::Result<Self> {
        // here we need to convert RouterOptions to Rust internal RouterConfig
        // the main logic is mainly refer to the construction function conversion logic in the Python binding
        
        let mode = config::RoutingMode::Regular {
            worker_urls: options.workerUrls,
        };

        let config_builder = config::RouterConfig::builder()
            .host(&options.host)
            .port(options.port)
            .mode(mode);
            
        // todo: continue to fill other configs ...

        let inner_config = config_builder.build().map_err(|e| {
             napi::Error::from_reason(format!("Config error: {}", e))
        })?;

        Ok(Router { inner_config })
    }

    #[napi]
    pub async fn start(&self) -> napi::Result<()> {
        let server_config = server::ServerConfig {
            host: self.inner_config.host.clone(),
            port: self.inner_config.port,
            router_config: self.inner_config.clone(),
            ..Default::default(), // other configs need to be filled completely
        };

        // call the core startup logic of sgl-router
        server::startup(server_config)
            .await
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }
}
