//! The end-to-end loop: watch two pools -> simulate a 2-leg swap ->
//! submit -> reconcile -> check halt. See `README.md` for the full
//! picture, including "Known limitations" and "Implementation notes"
//! for what is and isn't exercised against a live network.

use std::error::Error;
use std::time::Duration;

use k256::ecdsa::SigningKey;

use driftbrake_core::{HaltDecision, HaltPolicy, ProfitDecoder, ReconcileHistory, SimEngine};
use driftbrake_receipt_poller::{PollOutcome, PollerConfig, ReceiptPoller};
use driftbrake_reconcile::ReconcilePolicy;
use driftbrake_revm_backend::{BlockContext, CandidateTx, RevmBackend};

use toy_arbitrage::abi::{encode_arb_call, function_selector};
use toy_arbitrage::decoder::{
    DriftInjectingDecoder, ToyArbProfitDecoder, ToyArbRealizedDecoder, EXECUTE_ARB_SIGNATURE,
};
use toy_arbitrage::pools;
use toy_arbitrage::rpc::{
    JsonRpcClient, RpcDbFactory, RpcReceiptSource, RpcTxSubmitter, TxSubmitter, UnsignedTx,
};

/// **Illustrative placeholders — see README.md.** This example does not
/// ship a deployed executor contract or a specific pool pair; point
/// these at your own testnet deployment before running.
mod config {
    pub const EXECUTOR_ADDRESS: [u8; 20] = [0x99; 20];
    pub const POOL_A: [u8; 20] = [0xA1; 20];
    pub const POOL_B: [u8; 20] = [0xB2; 20];
    /// Notional amount (in the pair's smallest unit) simulated per trade.
    pub const TRADE_NOTIONAL: u128 = 1_000_000_000_000_000_000; // 1e18
    /// Minimum relative price divergence between the two pools before a
    /// trade is even simulated (2%) — a cheap pre-filter ahead of the
    /// far more expensive REVM simulation.
    pub const MIN_DIVERGENCE: f64 = 0.02;
    pub const GAS_LIMIT: u64 = 500_000;
}

struct CliArgs {
    inject_drift: bool,
    drift_after: u64,
}

fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let inject_drift = args.iter().any(|a| a == "--inject-drift");
    let drift_after = args
        .iter()
        .position(|a| a == "--drift-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    CliArgs {
        inject_drift,
        drift_after,
    }
}

/// Ethereum address derived from a secp256k1 public key: the low 20
/// bytes of `keccak256(uncompressed_pubkey[1..])`.
fn address_from_signing_key(key: &SigningKey) -> [u8; 20] {
    let verifying_key = key.verifying_key();
    let encoded = verifying_key.to_encoded_point(false); // uncompressed, 0x04 || X || Y
    let hash = driftbrake_revm_backend::keccak256(&encoded.as_bytes()[1..]);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

fn hex_to_bytes32(s: &str) -> Result<[u8; 32], Box<dyn Error>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()?;
    if bytes.len() != 32 {
        return Err("PRIVATE_KEY must be exactly 32 bytes of hex".into());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_dotenv();
    let cli = parse_args();

    let rpc_url =
        std::env::var("TESTNET_RPC_URL").map_err(|_| "set TESTNET_RPC_URL — see .env.example")?;
    let private_key_hex =
        std::env::var("PRIVATE_KEY").map_err(|_| "set PRIVATE_KEY — see .env.example")?;
    let chain_id: u64 = std::env::var("CHAIN_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(11155111); // Sepolia
    let block_time_ms: u64 = std::env::var("BLOCK_TIME_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12_000);

    let client = JsonRpcClient::new(rpc_url);
    let signing_key = SigningKey::from_bytes((&hex_to_bytes32(&private_key_hex)?).into())?;
    let caller = address_from_signing_key(&signing_key);

    let block = fetch_block_context(&client)?;
    let db_factory = RpcDbFactory::new(client.clone(), format!("0x{:x}", block.number));
    let engine = RevmBackend::new(
        db_factory,
        block,
        block_time_ms,
        /* timeout_fraction */ 0.5,
        /* concurrency */ 4,
    );

    let predicted_decoder = ToyArbProfitDecoder;
    let (drift_after, damping) = if cli.inject_drift {
        (cli.drift_after, 0.3)
    } else {
        (u64::MAX, 1.0)
    };
    let realized_decoder = DriftInjectingDecoder::new(ToyArbRealizedDecoder, drift_after, damping);

    let receipt_source = RpcReceiptSource::new(client.clone());
    let poller_config = PollerConfig::new(block_time_ms, 0.5, 10);
    let poller = ReceiptPoller::new(receipt_source, realized_decoder, poller_config);

    let submitter = RpcTxSubmitter::new(client.clone(), signing_key);
    let mut nonce = fetch_nonce(&client, caller)?;
    let gas_price = fetch_gas_price(&client)?;

    let mut history = ReconcileHistory::new();
    let mut halt_policy = ReconcilePolicy::default_dual_guard();

    println!(
        "toy-arbitrage: watching pools {:?} / {:?} via executor {:?}",
        config::POOL_A,
        config::POOL_B,
        config::EXECUTOR_ADDRESS
    );
    if cli.inject_drift {
        println!("--inject-drift enabled: realized profit will be dampened to {damping}x after {drift_after} trades");
    }

    loop {
        let reserves_a = pools::read_reserves(&engine, caller, config::POOL_A).await?;
        let reserves_b = pools::read_reserves(&engine, caller, config::POOL_B).await?;
        let divergence = pools::price_divergence(reserves_a, reserves_b);

        if divergence > config::MIN_DIVERGENCE {
            let (buy_pool, sell_pool) =
                if reserves_a.price_token0_in_token1() < reserves_b.price_token0_in_token1() {
                    (config::POOL_A, config::POOL_B)
                } else {
                    (config::POOL_B, config::POOL_A)
                };

            let selector = function_selector(EXECUTE_ARB_SIGNATURE);
            let calldata = encode_arb_call(selector, buy_pool, sell_pool, config::TRADE_NOTIONAL);
            let candidate = CandidateTx {
                caller,
                to: config::EXECUTOR_ADDRESS,
                value: 0,
                data: calldata.clone(),
                gas_limit: config::GAS_LIMIT,
            };

            let raw = engine.simulate(&candidate).await?;
            match predicted_decoder.decode_predicted(&raw) {
                Ok(predicted) if predicted.0 > 0 => {
                    println!("simulated profitable trade: predicted={predicted:?}, divergence={divergence:.4}");

                    let unsigned = UnsignedTx {
                        nonce,
                        gas_price,
                        gas_limit: config::GAS_LIMIT,
                        to: config::EXECUTOR_ADDRESS,
                        value: 0,
                        data: calldata,
                        chain_id,
                    };
                    let tx_hash = submitter.submit(&unsigned).await?;
                    nonce += 1;
                    println!("submitted tx {}", format_hash(&tx_hash));

                    let outcome = poller
                        .poll_until_settled(tx_hash, predicted, &mut history)
                        .await?;
                    println!("settlement outcome: {outcome:?}");

                    if matches!(outcome, PollOutcome::Realized(_)) {
                        if let HaltDecision::Halt(reason) = halt_policy.evaluate(&history) {
                            println!("HALT: {reason:?}");
                            break;
                        }
                    }
                }
                Ok(unprofitable) => {
                    println!("simulated trade not profitable ({unprofitable:?}), skipping");
                }
                Err(e) => {
                    eprintln!("decode_predicted error: {e}");
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(block_time_ms)).await;
    }

    Ok(())
}

fn format_hash(hash: &[u8; 32]) -> String {
    use std::fmt::Write;
    hash.iter().fold(String::from("0x"), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn fetch_block_context(client: &JsonRpcClient) -> Result<BlockContext, Box<dyn Error>> {
    let block = client.call("eth_getBlockByNumber", serde_json::json!(["latest", false]))?;
    let get_u64 = |field: &str| -> Result<u64, Box<dyn Error>> {
        let hex = block
            .get(field)
            .and_then(|v| v.as_str())
            .ok_or(format!("block response missing {field}"))?;
        Ok(u64::from_str_radix(hex.trim_start_matches("0x"), 16)?)
    };
    let get_u128 = |field: &str| -> Result<u128, Box<dyn Error>> {
        let hex = block.get(field).and_then(|v| v.as_str()).unwrap_or("0x0");
        Ok(u128::from_str_radix(hex.trim_start_matches("0x"), 16)?)
    };
    Ok(BlockContext {
        number: get_u64("number")?,
        timestamp: get_u64("timestamp")?,
        gas_limit: get_u64("gasLimit")?,
        basefee: get_u128("baseFeePerGas").unwrap_or(0),
        coinbase: [0u8; 20],
    })
}

fn fetch_nonce(client: &JsonRpcClient, address: [u8; 20]) -> Result<u64, Box<dyn Error>> {
    use std::fmt::Write;
    let addr_hex = address.iter().fold(String::from("0x"), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    });
    let result = client.call(
        "eth_getTransactionCount",
        serde_json::json!([addr_hex, "pending"]),
    )?;
    let hex = result
        .as_str()
        .ok_or("eth_getTransactionCount did not return a string")?;
    Ok(u64::from_str_radix(hex.trim_start_matches("0x"), 16)?)
}

fn fetch_gas_price(client: &JsonRpcClient) -> Result<u128, Box<dyn Error>> {
    let result = client.call("eth_gasPrice", serde_json::json!([]))?;
    let hex = result
        .as_str()
        .ok_or("eth_gasPrice did not return a string")?;
    Ok(u128::from_str_radix(hex.trim_start_matches("0x"), 16)?)
}

/// Minimal `.env` loader: reads `KEY=VALUE` lines from `./.env` if
/// present and sets them via `std::env::set_var`, without overwriting a
/// variable already set in the real environment. Not a general `.env`
/// parser (no quoting, escaping, or multiline values) — just enough for
/// this example's `TESTNET_RPC_URL` / `PRIVATE_KEY` / `CHAIN_ID` /
/// `BLOCK_TIME_MS`.
fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if std::env::var(key).is_err() {
                // SAFETY: single-threaded at this point in startup, before
                // the tokio runtime spawns any other tasks.
                #[allow(unused_unsafe)]
                unsafe {
                    std::env::set_var(key, value.trim());
                }
            }
        }
    }
}
