use std::str::FromStr;

use ethers::abi::{Address, Uint};
use ethers::types::{Bytes, Log};
use foundry_evm::CallKind;
use revm::Return;
use serde::{Deserialize, Serialize};
use warp::reject::custom;
use warp::reply::Json;
use warp::Rejection;

use crate::errors::{
    FromDecStrError, FromHexError, MultipleBlockNumbersError, MultipleChainIdsError,
    NoURLForChainIdError,
};

use super::config::Config;
use super::evm::Evm;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationRequest {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub from: Address,
    pub to: Address,
    pub data: Option<Bytes>,
    #[serde(rename = "gasLimit")]
    pub gas_limit: u64,
    pub value: Option<String>,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<u64>,
    #[serde(rename = "formatTrace")]
    pub format_trace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SimulationResponse {
    #[serde(rename = "simulationId")]
    pub simulation_id: u64,
    #[serde(rename = "gasUsed")]
    pub gas_used: u64,
    #[serde(rename = "blockNumber")]
    pub block_number: u64,
    pub success: bool,
    pub trace: Vec<CallTrace>,
    #[serde(rename = "formattedTrace")]
    pub formatted_trace: Option<String>,
    pub logs: Vec<Log>,
    #[serde(rename = "exitReason")]
    pub exit_reason: Return,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallTrace {
    #[serde(rename = "callType")]
    pub call_type: CallKind,
    pub from: Address,
    pub to: Address,
    pub value: Uint,
}

fn chain_id_to_fork_url(chain_id: u64, alchemy_key: String) -> Result<String, Rejection> {
    match chain_id {
        // ethereum
        1 => Ok(format!(
            "https://eth-mainnet.g.alchemy.com/v2/{alchemy_key}"
        )),
        5 => Ok(format!("https://eth-goerli.g.alchemy.com/v2/{alchemy_key}")),
        // polygon
        137 => Ok(format!(
            "https://polygon-mainnet.g.alchemy.com/v2/{alchemy_key}"
        )),
        80001 => Ok(format!(
            "https://polygon-mumbai.g.alchemy.com/v2/{alchemy_key}"
        )),
        // avalanche
        43114 => Ok("https://api.avax.network/ext/bc/C/rpc".to_string()),
        43113 => Ok("https://api.avax-test.network/ext/bc/C/rpc".to_string()),
        // fantom
        250 => Ok("https://rpcapi.fantom.network/".to_string()),
        4002 => Ok("https://rpc.testnet.fantom.network/".to_string()),
        // xdai
        100 => Ok("https://rpc.xdaichain.com/".to_string()),
        // bsc
        56 => Ok("https://bsc-dataseed.binance.org/".to_string()),
        97 => Ok("https://data-seed-prebsc-1-s1.binance.org:8545/".to_string()),
        // arbitrum
        42161 => Ok("https://arb1.arbitrum.io/rpc".to_string()),
        421613 => Ok("https://goerli-rollup.arbitrum.io/rpc".to_string()),
        // optimism
        10 => Ok("https://mainnet.optimism.io/".to_string()),
        420 => Ok("https://goerli.optimism.io/".to_string()),
        _ => Err(NoURLForChainIdError.into()),
    }
}

async fn run(
    evm: &mut Evm,
    transaction: SimulationRequest,
    commit: bool,
) -> Result<SimulationResponse, Rejection> {
    // Accept value in hex or decimal formats
    let value = if let Some(value) = transaction.value {
        if value.starts_with("0x") {
            Some(Uint::from_str(value.as_str()).map_err(|_err| custom(FromHexError))?)
        } else {
            Some(Uint::from_dec_str(value.as_str()).map_err(|_err| custom(FromDecStrError))?)
        }
    } else {
        None
    };

    let result = if commit {
        evm.call_raw_committing(
            transaction.from,
            transaction.to,
            value,
            transaction.data,
            transaction.gas_limit,
            transaction.format_trace.unwrap_or_default(),
        )
        .await?
    } else {
        evm.call_raw(
            transaction.from,
            transaction.to,
            value,
            transaction.data,
            transaction.format_trace.unwrap_or_default(),
        )
        .await?
    };

    Ok(SimulationResponse {
        simulation_id: 1,
        gas_used: result.gas_used,
        block_number: result.block_number,
        success: result.success,
        trace: result
            .trace
            .unwrap_or_default()
            .arena
            .into_iter()
            .map(CallTrace::from)
            .collect(),
        logs: result.logs,
        exit_reason: result.exit_reason,
        formatted_trace: result.formatted_trace,
    })
}

pub async fn simulate(transaction: SimulationRequest, config: Config) -> Result<Json, Rejection> {
    let alchemy_key = config.alchemy_key.clone();
    let fork_url = chain_id_to_fork_url(transaction.chain_id, alchemy_key)?;
    let mut evm = Evm::new(
        None,
        fork_url,
        transaction.block_number,
        transaction.gas_limit,
        true,
        config.etherscan_key,
    );

    let response = run(&mut evm, transaction, false).await?;

    Ok(warp::reply::json(&response))
}

pub async fn simulate_bundle(
    transactions: Vec<SimulationRequest>,
    config: Config,
) -> Result<Json, Rejection> {
    let first_chain_id = transactions[0].chain_id;
    let first_block_number = transactions[0].block_number;

    let alchemy_key = config.alchemy_key.clone();
    let fork_url = chain_id_to_fork_url(first_chain_id, alchemy_key)?;
    let mut evm = Evm::new(
        None,
        fork_url,
        first_block_number,
        transactions[0].gas_limit,
        true,
        config.etherscan_key,
    );

    let mut response = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        if transaction.chain_id != first_chain_id {
            return Err(warp::reject::custom(MultipleChainIdsError()));
        }
        if transaction.block_number != first_block_number {
            return Err(warp::reject::custom(MultipleBlockNumbersError()));
        }
        response.push(run(&mut evm, transaction, true).await?);
    }

    Ok(warp::reply::json(&response))
}
