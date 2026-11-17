//! `web3_dapp` — generic EVM contract interactions from caller-supplied
//! calldata. Logic lives in the compiled TinyWallet module; this module owns
//! the RPC controllers and their schemas. Agent tools use that same bus API.

pub mod schemas;
