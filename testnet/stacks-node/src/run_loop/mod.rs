pub mod l1_observer;
pub mod neon;

use crate::{BurnchainController, BurnchainTip, ChainTip};

use stacks::chainstate::stacks::db::StacksChainState;
use stacks::chainstate::stacks::{
    TransactionAuth, TransactionPayload, TransactionSpendingCondition,
};
use stacks::util::vrf::VRFPublicKey;

use stacks::vm::database::BurnStateDB;

macro_rules! info_blue {
    ($($arg:tt)*) => ({
        eprintln!("\x1b[0;96m{}\x1b[0m", format!($($arg)*));
    })
}

#[allow(unused_macros)]
macro_rules! info_yellow {
    ($($arg:tt)*) => ({
        eprintln!("\x1b[0;33m{}\x1b[0m", format!($($arg)*));
    })
}

macro_rules! info_green {
    ($($arg:tt)*) => ({
        eprintln!("\x1b[0;32m{}\x1b[0m", format!($($arg)*));
    })
}

pub struct RunLoopCallbacks {
    on_burn_chain_initialized: Option<fn(&mut Box<dyn BurnchainController>)>,
    on_new_burn_chain_state: Option<fn(u64, &BurnchainTip, &ChainTip)>,
    on_new_stacks_chain_state:
        Option<fn(u64, &BurnchainTip, &ChainTip, &mut StacksChainState, &dyn BurnStateDB)>,
}

impl RunLoopCallbacks {
    pub fn new() -> RunLoopCallbacks {
        RunLoopCallbacks {
            on_burn_chain_initialized: None,
            on_new_burn_chain_state: None,
            on_new_stacks_chain_state: None,
        }
    }

    pub fn on_burn_chain_initialized(&mut self, callback: fn(&mut Box<dyn BurnchainController>)) {
        self.on_burn_chain_initialized = Some(callback);
    }

    pub fn on_new_burn_chain_state(&mut self, callback: fn(u64, &BurnchainTip, &ChainTip)) {
        self.on_new_burn_chain_state = Some(callback);
    }

    pub fn on_new_stacks_chain_state(
        &mut self,
        callback: fn(u64, &BurnchainTip, &ChainTip, &mut StacksChainState, &dyn BurnStateDB),
    ) {
        self.on_new_stacks_chain_state = Some(callback);
    }

    pub fn invoke_burn_chain_initialized(&self, burnchain: &mut Box<dyn BurnchainController>) {
        if let Some(cb) = self.on_burn_chain_initialized {
            cb(burnchain);
        }
    }

    pub fn invoke_new_burn_chain_state(
        &self,
        round: u64,
        burnchain_tip: &BurnchainTip,
        chain_tip: &ChainTip,
    ) {
        info_blue!(
            "Subnet: Burnchain block #{} ({}) was produced with sortition #{}",
            burnchain_tip.block_snapshot.block_height,
            burnchain_tip.block_snapshot.burn_header_hash,
            burnchain_tip.block_snapshot.sortition_hash
        );

        if let Some(cb) = self.on_new_burn_chain_state {
            cb(round, burnchain_tip, chain_tip);
        }
    }

    pub fn invoke_new_stacks_chain_state(
        &self,
        round: u64,
        burnchain_tip: &BurnchainTip,
        chain_tip: &ChainTip,
        chain_state: &mut StacksChainState,
        burn_dbconn: &dyn BurnStateDB,
    ) {
        info_green!(
            "Subnet: Stacks block #{} ({}) successfully produced, including {} transactions",
            chain_tip.metadata.stacks_block_height,
            chain_tip.metadata.index_block_hash(),
            chain_tip.block.txs.len()
        );
        for tx in chain_tip.block.txs.iter() {
            match &tx.auth {
                TransactionAuth::Standard(TransactionSpendingCondition::Singlesig(auth)) => {
                    println!(
                        "-> Tx issued by {:?} (fee: {}, nonce: {})",
                        auth.signer, auth.tx_fee, auth.nonce
                    )
                }
                _ => println!("-> Tx {:?}", tx.auth),
            }
            match &tx.payload {
                TransactionPayload::Coinbase(_) => println!("   Coinbase"),
                TransactionPayload::SmartContract(contract) => println!("   Publish smart contract\n**************************\n{:?}\n**************************", contract.code_body),
                TransactionPayload::TokenTransfer(recipent, amount, _) => println!("   Transfering {} µSTX to {}", amount, recipent.to_string()),
                _ => println!("   {:?}", tx.payload)
            }
        }

        if let Some(cb) = self.on_new_stacks_chain_state {
            cb(round, burnchain_tip, chain_tip, chain_state, burn_dbconn);
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegisteredKey {
    pub block_height: u64,
    pub op_vtxindex: u32,
    pub vrf_public_key: VRFPublicKey,
}
