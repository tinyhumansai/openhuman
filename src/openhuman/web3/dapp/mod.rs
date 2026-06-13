//! `web3_dapp` — generic EVM contract interactions from caller-supplied
//! calldata. Logic lives in the shared [`super::ops`]/[`super::store`]; this
//! module owns the RPC controllers and agent tools.

pub mod schemas;
pub mod tools;

pub use tools::{Web3DappCallTool, Web3DappExecuteTool};
