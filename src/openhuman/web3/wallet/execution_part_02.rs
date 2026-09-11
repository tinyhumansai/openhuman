
pub async fn balances() -> Result<RpcOutcome<Vec<BalanceInfo>>, String> {
    let status = wallet_status().await?.value;
    if !status.configured {
        return Err(WALLET_NOT_CONFIGURED_MESSAGE.to_string());
    }
    let mut out = Vec::with_capacity(status.accounts.len() + EVM_BALANCE_NETWORKS.len());
    for account in &status.accounts {
        match account.chain {
            // The EVM account fans out into one native-balance row per displayed
            // network (Ethereum, Base, BNB Chain), all sharing the same address.
            WalletChain::Evm => {
                for network in EVM_BALANCE_NETWORKS {
                    let asset = evm_native_asset(network)?;
                    let (raw, provider_status) = match chain_evm::evm_balance(
                        network,
                        &account.address,
                    )
                    .await
                    {
                        Ok(balance) => (balance.to_string(), ProviderStatus::Ready),
                        Err(error) => {
                            warn!(
                                    "{LOG_PREFIX} balances chain=evm network={} address={} falling back to zero: {error}",
                                    network.as_str(),
                                    account.address
                                );
                            ("0".to_string(), ProviderStatus::Missing)
                        }
                    };
                    out.push(balance_row(
                        WalletChain::Evm,
                        Some(network),
                        &account.address,
                        asset,
                        raw,
                        provider_status,
                    ));
                }
            }
            WalletChain::Btc => {
                let (raw, provider_status) = match chain_btc::native_balance(&account.address).await
                {
                    Ok(sats) => (sats.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                            "{LOG_PREFIX} balances chain=btc address={} falling back to zero: {error}",
                            account.address
                        );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Btc,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Btc)?,
                    raw,
                    provider_status,
                ));
            }
            WalletChain::Solana => {
                let (raw, provider_status) = match chain_sol::native_balance(&account.address).await
                {
                    Ok(lamports) => (lamports.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                                "{LOG_PREFIX} balances chain=solana address={} falling back to zero: {error}",
                                account.address
                            );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Solana,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Solana)?,
                    raw,
                    provider_status,
                ));
            }
            WalletChain::Tron => {
                let (raw, provider_status) = match chain_tron::native_balance(&account.address)
                    .await
                {
                    Ok(sun) => (sun.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                            "{LOG_PREFIX} balances chain=tron address={} falling back to zero: {error}",
                            account.address
                        );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Tron,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Tron)?,
                    raw,
                    provider_status,
                ));
            }
        }
    }
    debug!("{LOG_PREFIX} balances returned rows={}", out.len());
    Ok(RpcOutcome::new(
        out,
        vec!["wallet balances listed".to_string()],
    ))
}

pub async fn prepare_transfer(
    params: PrepareTransferParams,
) -> Result<RpcOutcome<PreparedTransaction>, String> {
    let to = validate_address(params.chain, &params.to_address)?;
    let amount = validate_amount(&params.amount_raw)?;
    if amount == 0 {
        return Err("transfer amount must be greater than zero".to_string());
    }
    let network = if params.chain == WalletChain::Evm {
        Some(params.evm_network.unwrap_or(EvmNetwork::EthereumMainnet))
    } else {
        None
    };
    let account = require_account(params.chain).await?;
    let asset = match params.asset_symbol.as_deref().map(str::trim) {
        None | Some("") => {
            // native asset for the chain (or chosen EVM network).
            let catalog = if let Some(net) = network {
                evm_asset_catalog(net)
            } else {
                super::defaults::asset_catalog(params.chain)
            };
            catalog
                .into_iter()
                .find(|value| value.native)
                .ok_or_else(|| {
                    format!(
                        "native asset metadata missing for '{}'",
                        chain_str(params.chain)
                    )
                })?
        }
        Some(symbol) => find_asset_for_network(params.chain, network, symbol).ok_or_else(|| {
            format!(
                "unsupported asset_symbol '{symbol}' for chain '{}'",
                chain_str(params.chain)
            )
        })?,
    };
    let kind = if asset.native {
        PreparedKind::NativeTransfer
    } else {
        PreparedKind::TokenTransfer
    };
    // BTC has no native token concept; reject TokenTransfer on btc.
    if matches!(params.chain, WalletChain::Btc) && !asset.native {
        return Err("token transfers are not supported on Bitcoin".to_string());
    }
    let now = now_ms();
    let label = if let Some(net) = network {
        format!("{} ({})", chain_str(params.chain), net.network_label())
    } else {
        chain_str(params.chain).to_string()
    };
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind,
        chain: params.chain,
        evm_network: network,
        from_address: account.address.clone(),
        to_address: to,
        asset_symbol: asset.symbol.clone(),
        amount_raw: amount.to_string(),
        amount_formatted: format_amount(amount, asset.decimals),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: asset.contract_address.clone(),
        estimated_fee_raw: estimated_fee_raw(params.chain, kind),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + QUOTE_TTL_MS,
        notes: vec![format!(
            "Prepared {} transfer on {} using default network settings.",
            asset.symbol, label
        )],
        owner: current_owner(),
    };
    debug!(
        "{LOG_PREFIX} prepare_transfer chain={} kind={:?} quote_id={} amount={} asset={}",
        chain_str(params.chain),
        kind,
        quote.quote_id,
        quote.amount_raw,
        quote.asset_symbol
    );
    Ok(RpcOutcome::new(
        store_quote(quote),
        vec!["wallet transfer prepared".to_string()],
    ))
}

/// Resolve the EVM network for a tx read, defaulting to Ethereum mainnet.
fn read_network(chain: WalletChain, evm_network: Option<EvmNetwork>) -> Option<EvmNetwork> {
    if chain == WalletChain::Evm {
        Some(evm_network.unwrap_or(EvmNetwork::EthereumMainnet))
    } else {
        None
    }
}

/// Check the on-chain lifecycle state of a previously broadcast transaction.
pub async fn tx_status(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxStatusInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::tx_status(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::tx_status(hash).await?,
        WalletChain::Solana => chain_sol::tx_status(hash).await?,
        WalletChain::Tron => chain_tron::tx_status(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} tx_status chain={} hash={} state={:?}",
        chain_str(chain),
        hash,
        info.state
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx status fetched".to_string()],
    ))
}

/// Fetch the receipt of a broadcast transaction (success flag, fee, block).
pub async fn tx_receipt(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxReceiptInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::tx_receipt(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::tx_receipt(hash).await?,
        WalletChain::Solana => chain_sol::tx_receipt(hash).await?,
        WalletChain::Tron => chain_tron::tx_receipt(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} tx_receipt chain={} hash={} found={}",
        chain_str(chain),
        hash,
        info.found
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx receipt fetched".to_string()],
    ))
}

/// Look up the raw transaction payload by hash.
pub async fn lookup_tx(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxLookupInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::lookup_tx(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::lookup_tx(hash).await?,
        WalletChain::Solana => chain_sol::lookup_tx(hash).await?,
        WalletChain::Tron => chain_tron::lookup_tx(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} lookup_tx chain={} hash={} found={}",
        chain_str(chain),
        hash,
        info.found
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx looked up".to_string()],
    ))
}

/// Crate-internal: sign+broadcast an externally-built unsigned EVM transaction
/// (deBridge swap/bridge or generic dapp calldata). See [`chain_evm::sign_and_broadcast_evm`].
pub(crate) async fn sign_and_broadcast_evm(
    network: EvmNetwork,
    to: &str,
    data_hex: Option<String>,
    value_raw: &str,
) -> Result<RawBroadcastResult, String> {
    chain_evm::sign_and_broadcast_evm(network, to, data_hex, value_raw).await
}

/// Crate-internal: sign+broadcast an externally-built hex `VersionedTransaction`
/// (deBridge Solana swap/bridge). See [`chain_sol::sign_and_broadcast_versioned`].
pub(crate) async fn sign_and_broadcast_solana(
    tx_blob_hex: &str,
) -> Result<RawBroadcastResult, String> {
    chain_sol::sign_and_broadcast_versioned(tx_blob_hex).await
}

pub async fn execute_prepared(
    params: ExecutePreparedParams,
) -> Result<RpcOutcome<ExecutionResult>, String> {
    if !params.confirmed {
        return Err("execute_prepared requires `confirmed: true`".to_string());
    }
    // Bind execute to the chat-thread that prepared the quote.
    // `current_owner()` returns the caller's `APPROVAL_CHAT_CONTEXT` (or `None`
    // for non-chat callers). `take_quote_for` enforces equality with the
    // prepare-time owner and returns the same "not found" error on mismatch
    // — leaked `quote_id`s in a shared channel cannot be hijacked from a
    // different agent session.
    let caller = current_owner();
    // Atomically remove the quote *before* broadcasting so two concurrent
    // confirmations can't both pass get_quote() and double-submit. If signing
    // or broadcast fails we restore the quote to keep it retryable.
    let quote = take_quote_for(&params.quote_id, caller)?;
    let chain = quote.chain;
    let restorable = quote.clone();
    let result = match chain {
        WalletChain::Evm => chain_evm::execute_evm_quote(quote).await,
        WalletChain::Btc => chain_btc::execute_btc_quote(quote).await,
        WalletChain::Solana => chain_sol::execute_solana_quote(quote).await,
        WalletChain::Tron => chain_tron::execute_tron_quote(quote).await,
    };
    let result = match result {
        Ok(value) => value,
        Err(error) => {
            // Restore the quote so the caller can fix the cause and retry.
            // Refresh the TTL window so a slow chain call (network timeouts
            // can chew through the original 5-min budget) doesn't hand back
            // an immediately-expired quote.
            let mut refreshed = restorable;
            let now = now_ms();
            refreshed.expires_at_ms = now + QUOTE_TTL_MS;
            store_quote(refreshed);
            warn!(
                "{LOG_PREFIX} execute chain={} quote_id={} failed (quote restored, ttl refreshed): {error}",
                chain_str(chain),
                params.quote_id
            );
            return Err(error);
        }
    };
    let explorer_fallback = explorer_tx_url(chain, &result.transaction_hash);
    let mut final_result = result;
    if final_result.explorer_url.is_none() {
        final_result.explorer_url = explorer_fallback;
    }
    Ok(RpcOutcome::new(
        final_result,
        vec!["wallet transaction broadcast".to_string()],
    ))
}
