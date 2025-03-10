// Copyright (C) 2013-2021 Blockstack PBC, a public benefit corporation
// Copyright (C) 2020 Stacks Open Internet Foundation
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::cmp;
use std::collections::HashSet;
use std::io;

use crate::burnchains::Address;
use crate::burnchains::Txid;
use crate::chainstate::burn::ConsensusHash;
use crate::chainstate::stacks::db::test::chainstate_path;
use crate::chainstate::stacks::db::test::instantiate_chainstate;
use crate::chainstate::stacks::db::test::instantiate_chainstate_with_balances;
use crate::chainstate::stacks::db::StreamCursor;
use crate::chainstate::stacks::test::codec_all_transactions;
use crate::chainstate::stacks::{
    db::blocks::MemPoolRejection, db::StacksChainState, index::MarfTrieId, CoinbasePayload,
    Error as ChainstateError, SinglesigHashMode, SinglesigSpendingCondition, StacksPrivateKey,
    StacksPublicKey, StacksTransaction, StacksTransactionSigner, TokenTransferMemo,
    TransactionAnchorMode, TransactionAuth, TransactionContractCall, TransactionPayload,
    TransactionPostConditionMode, TransactionPublicKeyEncoding, TransactionSmartContract,
    TransactionSpendingCondition, TransactionVersion,
};
use crate::chainstate::stacks::{
    C32_ADDRESS_VERSION_MAINNET_SINGLESIG, C32_ADDRESS_VERSION_TESTNET_SINGLESIG,
};
use crate::core::mempool::MemPoolWalkSettings;
use crate::core::mempool::TxTag;
use crate::core::mempool::{BLOOM_COUNTER_DEPTH, BLOOM_COUNTER_ERROR_RATE, MAX_BLOOM_COUNTER_TXS};
use crate::core::FIRST_BURNCHAIN_CONSENSUS_HASH;
use crate::core::FIRST_STACKS_BLOCK_HASH;
use crate::net::Error as NetError;
use crate::net::HttpResponseType;
use crate::net::MemPoolSyncData;
use crate::util_lib::bloom::test::setup_bloom_counter;
use crate::util_lib::bloom::*;
use crate::util_lib::db::{tx_begin_immediate, DBConn, FromRow};
use crate::util_lib::strings::StacksString;
use clarity::vm::{
    database::HeadersDB,
    errors::Error as ClarityError,
    errors::RuntimeErrorType,
    test_util::TEST_BURN_STATE_DB,
    types::{PrincipalData, QualifiedContractIdentifier},
    ClarityName, ContractName, Value,
};
use stacks_common::address::AddressHashMode;
use stacks_common::types::chainstate::TrieHash;
use stacks_common::util::get_epoch_time_ms;
use stacks_common::util::hash::Hash160;
use stacks_common::util::secp256k1::MessageSignature;
use stacks_common::util::{hash::hex_bytes, hash::to_hex, hash::*, log, secp256k1::*};

use crate::chainstate::stacks::index::TrieHashExtension;
use crate::chainstate::stacks::{StacksBlockHeader, StacksMicroblockHeader};
use crate::codec::StacksMessageCodec;
use crate::types::chainstate::{BlockHeaderHash, BurnchainHeaderHash};
use crate::types::chainstate::{StacksAddress, StacksBlockId, StacksWorkScore, VRFSeed};
use crate::{
    chainstate::stacks::db::StacksHeaderInfo, util::vrf::VRFProof, vm::costs::ExecutionCost,
};
use clarity::vm::types::StacksAddressExtensions;

use super::MemPoolDB;

use rand::prelude::*;
use rand::thread_rng;

use crate::chainstate::stacks::db::blocks::MessageSignatureList;
use stacks_common::codec::read_next;
use stacks_common::codec::Error as codec_error;

const FOO_CONTRACT: &'static str = "(define-public (foo) (ok 1))
                                    (define-public (bar (x uint)) (ok x))";
const SK_1: &'static str = "a1289f6438855da7decf9b61b852c882c398cff1446b2a0f823538aa2ebef92e01";
const SK_2: &'static str = "4ce9a8f7539ea93753a36405b16e8b57e15a552430410709c2b6d65dca5c02e201";
const SK_3: &'static str = "cb95ddd0fe18ec57f4f3533b95ae564b3f1ae063dbf75b46334bd86245aef78501";

#[test]
fn mempool_db_init() {
    let _chainstate = instantiate_chainstate(false, 0x80000000, "mempool_db_init");
    let chainstate_path = chainstate_path("mempool_db_init");
    let _mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();
}

#[cfg(test)]
fn make_block(
    chainstate: &mut StacksChainState,
    block_consensus: ConsensusHash,
    parent: &(ConsensusHash, BlockHeaderHash),
    burn_height: u64,
    block_height: u64,
) -> (ConsensusHash, BlockHeaderHash) {
    let (mut chainstate_tx, clar_tx) = chainstate.chainstate_tx_begin().unwrap();

    let anchored_header = StacksBlockHeader {
        version: 1,
        total_work: StacksWorkScore {
            work: block_height,
            burn: 1,
        },
        proof: VRFProof::empty(),
        parent_block: parent.1.clone(),
        parent_microblock: BlockHeaderHash([0; 32]),
        parent_microblock_sequence: 0,
        tx_merkle_root: Sha512Trunc256Sum::empty(),
        state_index_root: TrieHash::from_empty_data(),
        withdrawal_merkle_root: Sha512Trunc256Sum::empty(),
        microblock_pubkey_hash: Hash160([0; 20]),
        miner_signatures: MessageSignatureList::empty(),
    };

    let block_hash = anchored_header.block_hash();

    let c_tx = StacksChainState::chainstate_block_begin(
        &chainstate_tx,
        clar_tx,
        &TEST_BURN_STATE_DB,
        &parent.0,
        &parent.1,
        &block_consensus,
        &block_hash,
    );

    let new_tip_info = StacksHeaderInfo {
        anchored_header,
        microblock_tail: None,
        index_root: TrieHash::from_empty_data(),
        stacks_block_height: block_height,
        consensus_hash: block_consensus.clone(),
        burn_header_hash: BurnchainHeaderHash([0; 32]),
        burn_header_height: burn_height as u32,
        burn_header_timestamp: 0,
        anchored_block_size: 1,
        withdrawal_tree: MerkleTree::empty(),
    };

    c_tx.commit_block();

    let new_index_hash = StacksBlockId::new(&block_consensus, &block_hash);

    // instantiate the inner MARF
    chainstate_tx
        .put_indexed_all(
            &StacksBlockId::new(&parent.0, &parent.1),
            &new_index_hash,
            &vec![],
            &vec![],
        )
        .unwrap();

    StacksChainState::insert_stacks_block_header(
        &mut chainstate_tx,
        &new_index_hash,
        &new_tip_info,
        &ExecutionCost::zero(),
    )
    .unwrap();

    chainstate_tx.commit().unwrap();

    (block_consensus, block_hash)
}

#[test]
fn mempool_walk_over_fork() {
    let mut chainstate =
        instantiate_chainstate_with_balances(false, 0x80000000, "mempool_walk_over_fork", vec![]);

    // genesis -> b_1* -> b_2*
    //               \-> b_3 -> b_4
    //
    // *'d blocks accept transactions,
    //   try to walk at b_4, we should be able to find
    //   the transaction at b_1

    let b_1 = make_block(
        &mut chainstate,
        ConsensusHash([0x1; 20]),
        &(
            FIRST_BURNCHAIN_CONSENSUS_HASH.clone(),
            FIRST_STACKS_BLOCK_HASH.clone(),
        ),
        1,
        1,
    );
    let b_2 = make_block(&mut chainstate, ConsensusHash([0x2; 20]), &b_1, 2, 2);
    let b_5 = make_block(&mut chainstate, ConsensusHash([0x5; 20]), &b_2, 5, 3);
    let b_3 = make_block(&mut chainstate, ConsensusHash([0x3; 20]), &b_1, 3, 2);
    let b_4 = make_block(&mut chainstate, ConsensusHash([0x4; 20]), &b_3, 4, 3);

    let chainstate_path = chainstate_path("mempool_walk_over_fork");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let mut all_txs = codec_all_transactions(
        &TransactionVersion::Testnet,
        0x80000000,
        &TransactionAnchorMode::Any,
        &TransactionPostConditionMode::Allow,
    );

    let blocks_to_broadcast_in = [&b_1, &b_2, &b_4];
    let mut txs = [
        all_txs.pop().unwrap(),
        all_txs.pop().unwrap(),
        all_txs.pop().unwrap(),
    ];
    for tx in txs.iter_mut() {
        tx.set_tx_fee(123);
    }

    for ix in 0..3 {
        let mut mempool_tx = mempool.tx_begin().unwrap();

        let block = &blocks_to_broadcast_in[ix];
        let good_tx = &txs[ix];

        let origin_address = StacksAddress {
            version: 22,
            bytes: Hash160::from_data(&[ix as u8; 32]),
        };
        let sponsor_address = StacksAddress {
            version: 22,
            bytes: Hash160::from_data(&[0x80 | (ix as u8); 32]),
        };

        let txid = good_tx.txid();
        let tx_bytes = good_tx.serialize_to_vec();
        let tx_fee = good_tx.get_tx_fee();

        let height = 1 + ix as u64;

        let origin_nonce = 0; // (2 * ix + i) as u64;
        let sponsor_nonce = 0; // (2 * ix + i) as u64;

        assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &block.0,
            &block.1,
            txid,
            tx_bytes,
            tx_fee,
            height,
            &origin_address,
            origin_nonce,
            &sponsor_address,
            sponsor_nonce,
            None,
        )
        .unwrap();

        assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        mempool_tx.commit().unwrap();
    }

    // genesis -> b_1* -> b_2* -> b_5
    //               \-> b_3 -> b_4
    //
    // *'d blocks accept transactions,
    //   try to walk at b_4, we should be able to find
    //   the transaction at b_1

    let mut mempool_settings = MemPoolWalkSettings::default();
    mempool_settings.min_tx_fee = 10;

    chainstate.with_read_only_clarity_tx(
        &TEST_BURN_STATE_DB,
        &StacksBlockHeader::make_index_block_hash(&b_2.0, &b_2.1),
        |clarity_conn| {
            let mut count_txs = 0;
            mempool
                .iterate_candidates::<_, ChainstateError, _>(
                    clarity_conn,
                    2,
                    mempool_settings.clone(),
                    |_, available_tx, _| {
                        count_txs += 1;
                        Ok(true)
                    },
                )
                .unwrap();
            assert_eq!(
                count_txs, 3,
                "Mempool should find three transactions from b_2"
            );
        },
    );

    // Now that the mempool has iterated over those transactions, its view of the
    //  nonce for the origin address should have changed. Now it should find *no* transactions.
    chainstate.with_read_only_clarity_tx(
        &TEST_BURN_STATE_DB,
        &StacksBlockHeader::make_index_block_hash(&b_2.0, &b_2.1),
        |clarity_conn| {
            let mut count_txs = 0;
            mempool
                .iterate_candidates::<_, ChainstateError, _>(
                    clarity_conn,
                    2,
                    mempool_settings.clone(),
                    |_, available_tx, _| {
                        count_txs += 1;
                        Ok(true)
                    },
                )
                .unwrap();
            assert_eq!(count_txs, 0, "Mempool should find no transactions");
        },
    );

    mempool
        .reset_last_known_nonces()
        .expect("Should be able to reset nonces");

    chainstate.with_read_only_clarity_tx(
        &TEST_BURN_STATE_DB,
        &StacksBlockHeader::make_index_block_hash(&b_5.0, &b_5.1),
        |clarity_conn| {
            let mut count_txs = 0;
            mempool
                .iterate_candidates::<_, ChainstateError, _>(
                    clarity_conn,
                    3,
                    mempool_settings.clone(),
                    |_, available_tx, _| {
                        count_txs += 1;
                        Ok(true)
                    },
                )
                .unwrap();
            assert_eq!(
                count_txs, 3,
                "Mempool should find three transactions from b_5"
            );
        },
    );

    mempool
        .reset_last_known_nonces()
        .expect("Should be able to reset nonces");

    // The mempool iterator no longer does any consideration of what block accepted
    //  the transaction, so b_3 should have the same view.
    chainstate.with_read_only_clarity_tx(
        &TEST_BURN_STATE_DB,
        &StacksBlockHeader::make_index_block_hash(&b_3.0, &b_3.1),
        |clarity_conn| {
            let mut count_txs = 0;
            mempool
                .iterate_candidates::<_, ChainstateError, _>(
                    clarity_conn,
                    2,
                    mempool_settings.clone(),
                    |_, available_tx, _| {
                        count_txs += 1;
                        Ok(true)
                    },
                )
                .unwrap();
            assert_eq!(
                count_txs, 3,
                "Mempool should find three transactions from b_3"
            );
        },
    );

    mempool
        .reset_last_known_nonces()
        .expect("Should be able to reset nonces");

    chainstate.with_read_only_clarity_tx(
        &TEST_BURN_STATE_DB,
        &StacksBlockHeader::make_index_block_hash(&b_4.0, &b_4.1),
        |clarity_conn| {
            let mut count_txs = 0;
            mempool
                .iterate_candidates::<_, ChainstateError, _>(
                    clarity_conn,
                    3,
                    mempool_settings.clone(),
                    |_, available_tx, _| {
                        count_txs += 1;
                        Ok(true)
                    },
                )
                .unwrap();
            assert_eq!(
                count_txs, 3,
                "Mempool should find three transactions from b_4"
            );
        },
    );

    mempool
        .reset_last_known_nonces()
        .expect("Should be able to reset nonces");

    // let's test replace-across-fork while we're here.
    // first try to replace a tx in b_2 in b_1 - should fail because they are in the same fork
    let mut mempool_tx = mempool.tx_begin().unwrap();
    let block = &b_1;
    let tx = &txs[1];
    let origin_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[1; 32]),
    };
    let sponsor_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[0x81; 32]),
    };

    let txid = tx.txid();
    let tx_bytes = tx.serialize_to_vec();
    let tx_fee = tx.get_tx_fee();

    let height = 3;
    let origin_nonce = 0;
    let sponsor_nonce = 0;

    // make sure that we already have the transaction we're testing for replace-across-fork
    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    assert!(MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &block.0,
        &block.1,
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .is_err());

    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());
    mempool_tx.commit().unwrap();

    // now try replace-across-fork from b_2 to b_4
    // check that the number of transactions at b_2 and b_4 starts at 1 each
    assert_eq!(
        MemPoolDB::get_num_tx_at_block(&mempool.db, &b_4.0, &b_4.1).unwrap(),
        1
    );
    assert_eq!(
        MemPoolDB::get_num_tx_at_block(&mempool.db, &b_2.0, &b_2.1).unwrap(),
        1
    );
    let mut mempool_tx = mempool.tx_begin().unwrap();
    let block = &b_4;
    let tx = &txs[1];
    let origin_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[0; 32]),
    };
    let sponsor_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[1; 32]),
    };

    let txid = tx.txid();
    let tx_bytes = tx.serialize_to_vec();
    let tx_fee = tx.get_tx_fee();

    let height = 3;
    let origin_nonce = 1;
    let sponsor_nonce = 1;

    // make sure that we already have the transaction we're testing for replace-across-fork
    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &block.0,
        &block.1,
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .unwrap();

    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    mempool_tx.commit().unwrap();

    // after replace-across-fork, tx[1] should have moved from the b_2->b_5 fork to b_4
    assert_eq!(
        MemPoolDB::get_num_tx_at_block(&mempool.db, &b_4.0, &b_4.1).unwrap(),
        2
    );
    assert_eq!(
        MemPoolDB::get_num_tx_at_block(&mempool.db, &b_2.0, &b_2.1).unwrap(),
        0
    );
}

#[test]
fn mempool_do_not_replace_tx() {
    let mut chainstate = instantiate_chainstate_with_balances(
        false,
        0x80000000,
        "mempool_do_not_replace_tx",
        vec![],
    );

    // genesis -> b_1 -> b_2
    //      \-> b_3
    //
    let b_1 = make_block(
        &mut chainstate,
        ConsensusHash([0x1; 20]),
        &(
            FIRST_BURNCHAIN_CONSENSUS_HASH.clone(),
            FIRST_STACKS_BLOCK_HASH.clone(),
        ),
        1,
        1,
    );
    let b_2 = make_block(&mut chainstate, ConsensusHash([0x2; 20]), &b_1, 2, 2);
    let b_3 = make_block(&mut chainstate, ConsensusHash([0x3; 20]), &b_1, 1, 1);

    let chainstate_path = chainstate_path("mempool_do_not_replace_tx");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let mut txs = codec_all_transactions(
        &TransactionVersion::Testnet,
        0x80000000,
        &TransactionAnchorMode::Any,
        &TransactionPostConditionMode::Allow,
    );
    let mut tx = txs.pop().unwrap();

    let mut mempool_tx = mempool.tx_begin().unwrap();

    // do an initial insert
    let origin_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[0; 32]),
    };
    let sponsor_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&[1; 32]),
    };

    tx.set_tx_fee(123);

    // test insert
    let txid = tx.txid();
    let tx_bytes = tx.serialize_to_vec();

    let tx_fee = tx.get_tx_fee();
    let height = 100;

    let origin_nonce = tx.get_origin_nonce();
    let sponsor_nonce = match tx.get_sponsor_nonce() {
        Some(n) => n,
        None => origin_nonce,
    };

    assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &b_1.0,
        &b_1.1,
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .unwrap();

    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    let prior_txid = txid.clone();

    // now, let's try inserting again, with a lower fee, but at a different block hash
    tx.set_tx_fee(100);
    let txid = tx.txid();
    let tx_bytes = tx.serialize_to_vec();
    let tx_fee = tx.get_tx_fee();
    let height = 100;

    let err_resp = MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &b_2.0,
        &b_2.1,
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .unwrap_err();
    assert!(match err_resp {
        MemPoolRejection::ConflictingNonceInMempool => true,
        _ => false,
    });

    assert!(MemPoolDB::db_has_tx(&mempool_tx, &prior_txid).unwrap());
    assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());
}

#[test]
fn mempool_db_load_store_replace_tx() {
    let mut chainstate =
        instantiate_chainstate(false, 0x80000000, "mempool_db_load_store_replace_tx");
    let chainstate_path = chainstate_path("mempool_db_load_store_replace_tx");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let mut txs = codec_all_transactions(
        &TransactionVersion::Testnet,
        0x80000000,
        &TransactionAnchorMode::Any,
        &TransactionPostConditionMode::Allow,
    );
    let num_txs = txs.len() as u64;

    let mut mempool_tx = mempool.tx_begin().unwrap();

    eprintln!("add all txs");
    for (i, mut tx) in txs.drain(..).enumerate() {
        // make sure each address is unique per tx (not the case in codec_all_transactions)
        let origin_address = StacksAddress {
            version: 22,
            bytes: Hash160::from_data(&i.to_be_bytes()),
        };
        let sponsor_address = StacksAddress {
            version: 22,
            bytes: Hash160::from_data(&(i + 1).to_be_bytes()),
        };

        tx.set_tx_fee(123);

        // test insert

        let txid = tx.txid();
        let mut tx_bytes = vec![];
        tx.consensus_serialize(&mut tx_bytes).unwrap();
        let expected_tx = tx.clone();

        let tx_fee = tx.get_tx_fee();
        let height = 100;
        let origin_nonce = tx.get_origin_nonce();
        let sponsor_nonce = match tx.get_sponsor_nonce() {
            Some(n) => n,
            None => origin_nonce,
        };
        let len = tx_bytes.len() as u64;

        assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &ConsensusHash([0x1; 20]),
            &BlockHeaderHash([0x2; 32]),
            txid,
            tx_bytes,
            tx_fee,
            height,
            &origin_address,
            origin_nonce,
            &sponsor_address,
            sponsor_nonce,
            None,
        )
        .unwrap();

        assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        // test retrieval
        let tx_info_opt = MemPoolDB::get_tx(&mempool_tx, &txid).unwrap();
        let tx_info = tx_info_opt.unwrap();

        assert_eq!(tx_info.tx, expected_tx);
        assert_eq!(tx_info.metadata.len, len);
        assert_eq!(tx_info.metadata.tx_fee, 123);
        assert_eq!(tx_info.metadata.origin_address, origin_address);
        assert_eq!(tx_info.metadata.origin_nonce, origin_nonce);
        assert_eq!(tx_info.metadata.sponsor_address, sponsor_address);
        assert_eq!(tx_info.metadata.sponsor_nonce, sponsor_nonce);
        assert_eq!(tx_info.metadata.consensus_hash, ConsensusHash([0x1; 20]));
        assert_eq!(
            tx_info.metadata.block_header_hash,
            BlockHeaderHash([0x2; 32])
        );
        assert_eq!(tx_info.metadata.block_height, height);

        // test replace-by-fee with a higher fee
        let old_txid = txid;

        tx.set_tx_fee(124);
        assert!(txid != tx.txid());

        let txid = tx.txid();
        let mut tx_bytes = vec![];
        tx.consensus_serialize(&mut tx_bytes).unwrap();
        let expected_tx = tx.clone();
        let tx_fee = tx.get_tx_fee();

        assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        let tx_info_before =
            MemPoolDB::get_tx_metadata_by_address(&mempool_tx, true, &origin_address, origin_nonce)
                .unwrap()
                .unwrap();
        assert_eq!(tx_info_before, tx_info.metadata);

        MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &ConsensusHash([0x1; 20]),
            &BlockHeaderHash([0x2; 32]),
            txid,
            tx_bytes,
            tx_fee,
            height,
            &origin_address,
            origin_nonce,
            &sponsor_address,
            sponsor_nonce,
            None,
        )
        .unwrap();

        // was replaced
        assert!(!MemPoolDB::db_has_tx(&mempool_tx, &old_txid).unwrap());
        assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

        let tx_info_after =
            MemPoolDB::get_tx_metadata_by_address(&mempool_tx, true, &origin_address, origin_nonce)
                .unwrap()
                .unwrap();
        assert!(tx_info_after != tx_info.metadata);

        // test retrieval -- transaction should have been replaced because it has a higher
        // estimated fee
        let tx_info_opt = MemPoolDB::get_tx(&mempool_tx, &txid).unwrap();

        let tx_info = tx_info_opt.unwrap();
        assert_eq!(tx_info.metadata, tx_info_after);

        assert_eq!(tx_info.tx, expected_tx);
        assert_eq!(tx_info.metadata.len, len);
        assert_eq!(tx_info.metadata.tx_fee, 124);
        assert_eq!(tx_info.metadata.origin_address, origin_address);
        assert_eq!(tx_info.metadata.origin_nonce, origin_nonce);
        assert_eq!(tx_info.metadata.sponsor_address, sponsor_address);
        assert_eq!(tx_info.metadata.sponsor_nonce, sponsor_nonce);
        assert_eq!(tx_info.metadata.consensus_hash, ConsensusHash([0x1; 20]));
        assert_eq!(
            tx_info.metadata.block_header_hash,
            BlockHeaderHash([0x2; 32])
        );
        assert_eq!(tx_info.metadata.block_height, height);

        // test replace-by-fee with a lower fee
        let old_txid = txid;

        tx.set_tx_fee(122);
        assert!(txid != tx.txid());

        let txid = tx.txid();
        let mut tx_bytes = vec![];
        tx.consensus_serialize(&mut tx_bytes).unwrap();
        let _expected_tx = tx.clone();
        let tx_fee = tx.get_tx_fee();

        assert!(match MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &ConsensusHash([0x1; 20]),
            &BlockHeaderHash([0x2; 32]),
            txid,
            tx_bytes,
            tx_fee,
            height,
            &origin_address,
            origin_nonce,
            &sponsor_address,
            sponsor_nonce,
            None,
        )
        .unwrap_err()
        {
            MemPoolRejection::ConflictingNonceInMempool => true,
            _ => false,
        });

        // was NOT replaced
        assert!(MemPoolDB::db_has_tx(&mempool_tx, &old_txid).unwrap());
        assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());
    }
    mempool_tx.commit().unwrap();

    eprintln!("get all txs");
    let txs = MemPoolDB::get_txs_after(
        &mempool.db,
        &ConsensusHash([0x1; 20]),
        &BlockHeaderHash([0x2; 32]),
        0,
        num_txs,
    )
    .unwrap();
    assert_eq!(txs.len() as u64, num_txs);

    eprintln!("get empty txs");
    let txs = MemPoolDB::get_txs_after(
        &mempool.db,
        &ConsensusHash([0x1; 20]),
        &BlockHeaderHash([0x3; 32]),
        0,
        num_txs,
    )
    .unwrap();
    assert_eq!(txs.len(), 0);

    eprintln!("get empty txs");
    let txs = MemPoolDB::get_txs_after(
        &mempool.db,
        &ConsensusHash([0x2; 20]),
        &BlockHeaderHash([0x2; 32]),
        0,
        num_txs,
    )
    .unwrap();
    assert_eq!(txs.len(), 0);

    eprintln!("garbage-collect");
    let mut mempool_tx = mempool.tx_begin().unwrap();
    MemPoolDB::garbage_collect(&mut mempool_tx, 101, None).unwrap();
    mempool_tx.commit().unwrap();

    let txs = MemPoolDB::get_txs_after(
        &mempool.db,
        &ConsensusHash([0x1; 20]),
        &BlockHeaderHash([0x2; 32]),
        0,
        num_txs,
    )
    .unwrap();
    assert_eq!(txs.len(), 0);
}

#[test]
fn mempool_db_test_rbf() {
    let mut chainstate = instantiate_chainstate(false, 0x80000000, "mempool_db_test_rbf");
    let chainstate_path = chainstate_path("mempool_db_test_rbf");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    // create initial transaction
    let mut mempool_tx = mempool.tx_begin().unwrap();
    let spending_condition = TransactionSpendingCondition::Singlesig(SinglesigSpendingCondition {
        signer: Hash160([0x11; 20]),
        hash_mode: SinglesigHashMode::P2PKH,
        key_encoding: TransactionPublicKeyEncoding::Uncompressed,
        nonce: 123,
        tx_fee: 456,
        signature: MessageSignature::from_raw(&vec![0xff; 65]),
    });
    let stx_address = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };
    let payload = TransactionPayload::TokenTransfer(
        PrincipalData::from(QualifiedContractIdentifier {
            issuer: stx_address.into(),
            name: "hello-contract-name".into(),
        }),
        123,
        TokenTransferMemo([0u8; 34]),
    );
    let mut tx = StacksTransaction {
        version: TransactionVersion::Testnet,
        chain_id: 0x80000000,
        auth: TransactionAuth::Standard(spending_condition.clone()),
        anchor_mode: TransactionAnchorMode::Any,
        post_condition_mode: TransactionPostConditionMode::Allow,
        post_conditions: Vec::new(),
        payload,
    };

    let i: usize = 0;
    let origin_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&i.to_be_bytes()),
    };
    let sponsor_address = StacksAddress {
        version: 22,
        bytes: Hash160::from_data(&(i + 1).to_be_bytes()),
    };

    tx.set_tx_fee(123);
    let txid = tx.txid();
    let mut tx_bytes = vec![];
    tx.consensus_serialize(&mut tx_bytes).unwrap();
    let expected_tx = tx.clone();
    let tx_fee = tx.get_tx_fee();
    let height = 100;
    let origin_nonce = tx.get_origin_nonce();
    let sponsor_nonce = match tx.get_sponsor_nonce() {
        Some(n) => n,
        None => origin_nonce,
    };
    let first_len = tx_bytes.len() as u64;

    assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());
    MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &ConsensusHash([0x1; 20]),
        &BlockHeaderHash([0x2; 32]),
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .unwrap();
    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    // test retrieval of initial transaction
    let tx_info_opt = MemPoolDB::get_tx(&mempool_tx, &txid).unwrap();
    let tx_info = tx_info_opt.unwrap();

    // test replace-by-fee with a higher fee, where the payload is smaller
    let old_txid = txid;
    let old_tx_fee = tx_fee;

    tx.set_tx_fee(124);
    tx.payload =
        TransactionPayload::TokenTransfer(stx_address.into(), 123, TokenTransferMemo([0u8; 34]));
    assert!(txid != tx.txid());
    let txid = tx.txid();
    let mut tx_bytes = vec![];
    tx.consensus_serialize(&mut tx_bytes).unwrap();
    let expected_tx = tx.clone();
    let tx_fee = tx.get_tx_fee();
    let second_len = tx_bytes.len() as u64;

    // these asserts are to ensure we are using the fee directly, not the fee rate
    assert!(second_len < first_len);
    assert!(second_len * tx_fee < first_len * old_tx_fee);
    assert!(tx_fee > old_tx_fee);
    assert!(!MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    let tx_info_before =
        MemPoolDB::get_tx_metadata_by_address(&mempool_tx, true, &origin_address, origin_nonce)
            .unwrap()
            .unwrap();
    assert_eq!(tx_info_before, tx_info.metadata);

    MemPoolDB::try_add_tx(
        &mut mempool_tx,
        &mut chainstate,
        &ConsensusHash([0x1; 20]),
        &BlockHeaderHash([0x2; 32]),
        txid,
        tx_bytes,
        tx_fee,
        height,
        &origin_address,
        origin_nonce,
        &sponsor_address,
        sponsor_nonce,
        None,
    )
    .unwrap();

    // check that the transaction was replaced
    assert!(!MemPoolDB::db_has_tx(&mempool_tx, &old_txid).unwrap());
    assert!(MemPoolDB::db_has_tx(&mempool_tx, &txid).unwrap());

    let tx_info_after =
        MemPoolDB::get_tx_metadata_by_address(&mempool_tx, true, &origin_address, origin_nonce)
            .unwrap()
            .unwrap();
    assert!(tx_info_after != tx_info.metadata);

    // test retrieval -- transaction should have been replaced because it has a higher fee
    let tx_info_opt = MemPoolDB::get_tx(&mempool_tx, &txid).unwrap();
    let tx_info = tx_info_opt.unwrap();
    assert_eq!(tx_info.metadata, tx_info_after);
    assert_eq!(tx_info.metadata.len, second_len);
    assert_eq!(tx_info.metadata.tx_fee, 124);
}

#[test]
fn test_add_txs_bloom_filter() {
    let mut chainstate = instantiate_chainstate(false, 0x80000000, "mempool_add_txs_bloom_filter");
    let chainstate_path = chainstate_path("mempool_add_txs_bloom_filter");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };

    let mut all_txids: Vec<Vec<Txid>> = vec![];

    // none conflict
    for block_height in 10..(10 + 10 * BLOOM_COUNTER_DEPTH) {
        let mut txids: Vec<Txid> = vec![];
        let mut fp_count = 0;

        let bf = mempool.get_txid_bloom_filter().unwrap();
        let mut mempool_tx = mempool.tx_begin().unwrap();
        for i in 0..128 {
            let pk = StacksPrivateKey::new();
            let mut tx = StacksTransaction {
                version: TransactionVersion::Testnet,
                chain_id: 0x80000000,
                auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
                anchor_mode: TransactionAnchorMode::Any,
                post_condition_mode: TransactionPostConditionMode::Allow,
                post_conditions: vec![],
                payload: TransactionPayload::TokenTransfer(
                    addr.to_account_principal(),
                    (block_height + i * 128) as u64,
                    TokenTransferMemo([0u8; 34]),
                ),
            };
            tx.set_tx_fee(1000);
            tx.set_origin_nonce(0);

            let txid = tx.txid();
            let tx_bytes = tx.serialize_to_vec();
            let origin_addr = tx.origin_address();
            let origin_nonce = tx.get_origin_nonce();
            let sponsor_addr = tx.sponsor_address().unwrap_or(origin_addr.clone());
            let sponsor_nonce = tx.get_sponsor_nonce().unwrap_or(origin_nonce);
            let tx_fee = tx.get_tx_fee();

            // should succeed
            MemPoolDB::try_add_tx(
                &mut mempool_tx,
                &mut chainstate,
                &ConsensusHash([0x1 + (block_height as u8); 20]),
                &BlockHeaderHash([0x2 + (block_height as u8); 32]),
                txid,
                tx_bytes,
                tx_fee,
                block_height as u64,
                &origin_addr,
                origin_nonce,
                &sponsor_addr,
                sponsor_nonce,
                None,
            )
            .unwrap();

            if bf.contains_raw(&tx.txid().0) {
                fp_count += 1;
            }

            txids.push(txid);
        }

        mempool_tx.commit().unwrap();

        // nearly all txs should be new
        assert!((fp_count as f64) / (MAX_BLOOM_COUNTER_TXS as f64) <= BLOOM_COUNTER_ERROR_RATE);

        let bf = mempool.get_txid_bloom_filter().unwrap();
        for txid in txids.iter() {
            assert!(
                bf.contains_raw(&txid.0),
                "Bloom filter does not contain {}",
                &txid
            );
        }

        all_txids.push(txids);

        if block_height > 10 + BLOOM_COUNTER_DEPTH {
            let expired_block_height = block_height - BLOOM_COUNTER_DEPTH;
            let bf = mempool.get_txid_bloom_filter().unwrap();
            for i in 0..(block_height - 10 - BLOOM_COUNTER_DEPTH) {
                let txids = &all_txids[i];
                let mut fp_count = 0;
                for txid in txids {
                    if bf.contains_raw(&txid.0) {
                        fp_count += 1;
                    }
                }

                // these expired txids should mostly be absent
                assert!(
                    (fp_count as f64) / (MAX_BLOOM_COUNTER_TXS as f64) <= BLOOM_COUNTER_ERROR_RATE
                );
            }
        }
    }
}

#[test]
fn test_txtags() {
    let mut chainstate = instantiate_chainstate(false, 0x80000000, "mempool_txtags");
    let chainstate_path = chainstate_path("mempool_txtags");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };

    let mut seed = [0u8; 32];
    thread_rng().fill_bytes(&mut seed);

    let mut all_txtags: Vec<Vec<TxTag>> = vec![];

    for block_height in 10..(10 + 10 * BLOOM_COUNTER_DEPTH) {
        let mut txtags: Vec<TxTag> = vec![];

        let mut mempool_tx = mempool.tx_begin().unwrap();
        for i in 0..128 {
            let pk = StacksPrivateKey::new();
            let mut tx = StacksTransaction {
                version: TransactionVersion::Testnet,
                chain_id: 0x80000000,
                auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
                anchor_mode: TransactionAnchorMode::Any,
                post_condition_mode: TransactionPostConditionMode::Allow,
                post_conditions: vec![],
                payload: TransactionPayload::TokenTransfer(
                    addr.to_account_principal(),
                    (block_height + i * 128) as u64,
                    TokenTransferMemo([0u8; 34]),
                ),
            };
            tx.set_tx_fee(1000);
            tx.set_origin_nonce(0);

            let txid = tx.txid();
            let tx_bytes = tx.serialize_to_vec();
            let origin_addr = tx.origin_address();
            let origin_nonce = tx.get_origin_nonce();
            let sponsor_addr = tx.sponsor_address().unwrap_or(origin_addr.clone());
            let sponsor_nonce = tx.get_sponsor_nonce().unwrap_or(origin_nonce);
            let tx_fee = tx.get_tx_fee();

            let txtag = TxTag::from(&seed, &txid);

            // should succeed
            MemPoolDB::try_add_tx(
                &mut mempool_tx,
                &mut chainstate,
                &ConsensusHash([0x1 + (block_height as u8); 20]),
                &BlockHeaderHash([0x2 + (block_height as u8); 32]),
                txid,
                tx_bytes,
                tx_fee,
                block_height as u64,
                &origin_addr,
                origin_nonce,
                &sponsor_addr,
                sponsor_nonce,
                None,
            )
            .unwrap();

            txtags.push(txtag);
        }

        mempool_tx.commit().unwrap();
        all_txtags.push(txtags);

        if block_height - 10 >= BLOOM_COUNTER_DEPTH {
            assert_eq!(
                MemPoolDB::get_num_recent_txs(mempool.conn()).unwrap(),
                (BLOOM_COUNTER_DEPTH * 128) as u64
            );
        }

        let txtags = mempool.get_txtags(&seed).unwrap();
        let len_txtags = all_txtags.len();
        let last_txtags =
            &all_txtags[len_txtags.saturating_sub(BLOOM_COUNTER_DEPTH as usize)..len_txtags];

        let mut expected_txtag_set = HashSet::new();
        for txtags in last_txtags.iter() {
            for txtag in txtags.iter() {
                expected_txtag_set.insert(txtag.clone());
            }
        }

        assert_eq!(expected_txtag_set.len(), txtags.len());
        for txtag in txtags.into_iter() {
            assert!(expected_txtag_set.contains(&txtag));
        }
    }
}

#[test]
#[ignore]
fn test_make_mempool_sync_data() {
    let mut chainstate = instantiate_chainstate(false, 0x80000000, "make_mempool_sync_data");
    let chainstate_path = chainstate_path("make_mempool_sync_data");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };

    let mut txids = vec![];
    let mut nonrecent_fp_rates = vec![];
    for block_height in 10..(10 + BLOOM_COUNTER_DEPTH + 1) {
        for i in 0..((MAX_BLOOM_COUNTER_TXS + 128) as usize) {
            let mut mempool_tx = mempool.tx_begin().unwrap();
            for j in 0..128 {
                let pk = StacksPrivateKey::new();
                let mut tx = StacksTransaction {
                    version: TransactionVersion::Testnet,
                    chain_id: 0x80000000,
                    auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
                    anchor_mode: TransactionAnchorMode::Any,
                    post_condition_mode: TransactionPostConditionMode::Allow,
                    post_conditions: vec![],
                    payload: TransactionPayload::TokenTransfer(
                        addr.to_account_principal(),
                        123,
                        TokenTransferMemo([0u8; 34]),
                    ),
                };
                tx.set_tx_fee(1000);
                tx.set_origin_nonce(0);

                let txid = tx.txid();
                let tx_bytes = tx.serialize_to_vec();
                let origin_addr = tx.origin_address();
                let origin_nonce = tx.get_origin_nonce();
                let sponsor_addr = tx.sponsor_address().unwrap_or(origin_addr.clone());
                let sponsor_nonce = tx.get_sponsor_nonce().unwrap_or(origin_nonce);
                let tx_fee = tx.get_tx_fee();

                // should succeed
                MemPoolDB::try_add_tx(
                    &mut mempool_tx,
                    &mut chainstate,
                    &ConsensusHash([0x1 + (block_height as u8); 20]),
                    &BlockHeaderHash([0x2 + (block_height as u8); 32]),
                    txid.clone(),
                    tx_bytes,
                    tx_fee,
                    block_height as u64,
                    &origin_addr,
                    origin_nonce,
                    &sponsor_addr,
                    sponsor_nonce,
                    None,
                )
                .unwrap();

                txids.push(txid);
            }
            mempool_tx.commit().unwrap();

            let ts_1 = get_epoch_time_ms();
            let ms = mempool.make_mempool_sync_data().unwrap();
            let ts_2 = get_epoch_time_ms();
            eprintln!(
                "make_mempool_sync_data({}): {} ms",
                txids.len(),
                ts_2.saturating_sub(ts_1)
            );

            let mut present_count: u32 = 0;
            let mut absent_count: u32 = 0;
            let mut fp_count: u32 = 0;
            match ms {
                MemPoolSyncData::BloomFilter(ref bf) => {
                    eprintln!(
                        "bloomfilter({}); txids.len() == {}",
                        block_height,
                        txids.len()
                    );
                    let recent_txids = mempool.get_bloom_txids().unwrap();
                    assert!(recent_txids.len() <= MAX_BLOOM_COUNTER_TXS as usize);

                    let max_height = MemPoolDB::get_max_height(mempool.conn())
                        .unwrap()
                        .unwrap_or(0);
                    eprintln!(
                        "bloomfilter({}): recent_txids.len() == {}, max height is {}",
                        block_height,
                        recent_txids.len(),
                        max_height
                    );

                    let mut recent_set = HashSet::new();
                    let mut in_bf = 0;
                    for txid in recent_txids.iter() {
                        if bf.contains_raw(&txid.0) {
                            in_bf += 1;
                        }
                        recent_set.insert(txid.clone());
                    }

                    eprintln!("in bloom filter: {}", in_bf);
                    assert!(in_bf >= recent_txids.len());

                    for txid in txids.iter() {
                        if !recent_set.contains(&txid) && bf.contains_raw(&txid.0) {
                            fp_count += 1;
                        }
                        if bf.contains_raw(&txid.0) {
                            present_count += 1;
                        } else {
                            absent_count += 1;
                        }
                    }

                    // all recent transactions should be present
                    assert!(
                        present_count >= cmp::min(MAX_BLOOM_COUNTER_TXS.into(), txids.len() as u32)
                    );
                }
                MemPoolSyncData::TxTags(ref seed, ref tags) => {
                    eprintln!("txtags({}); txids.len() == {}", block_height, txids.len());
                    let recent_txids = mempool.get_bloom_txids().unwrap();

                    // all tags are present in the recent set
                    let mut recent_set = HashSet::new();
                    for txid in recent_txids {
                        recent_set.insert(TxTag::from(seed, &txid));
                    }

                    for tag in tags.iter() {
                        assert!(recent_set.contains(tag));
                    }
                }
            }

            let mut nonrecent_fp_rate = 0.0f64;
            let recent_txids = mempool.get_bloom_txids().unwrap();
            if recent_txids.len() < (present_count + absent_count) as usize {
                nonrecent_fp_rate = (fp_count as f64)
                    / ((present_count + absent_count - (recent_txids.len() as u32)) as f64);
                eprintln!(
                    "Nonrecent false positive rate: {} / ({} + {} - {} = {}) = {}",
                    fp_count,
                    present_count,
                    absent_count,
                    recent_txids.len(),
                    present_count + absent_count - (recent_txids.len() as u32),
                    nonrecent_fp_rate
                );
            }

            let total_count = MemPoolDB::get_num_recent_txs(&mempool.conn()).unwrap();
            eprintln!(
                "present_count: {}, absent count: {}, total sent: {}, total recent: {}",
                present_count,
                absent_count,
                txids.len(),
                total_count
            );

            nonrecent_fp_rates.push(nonrecent_fp_rate);
        }
    }

    // average false positive rate for non-recent transactions should be around the bloom
    // counter false positive rate
    let num_nonrecent_fp_samples = nonrecent_fp_rates.len() as f64;
    let avg_nonrecent_fp_rate =
        nonrecent_fp_rates.iter().fold(0.0f64, |acc, x| acc + x) / num_nonrecent_fp_samples;

    assert!((avg_nonrecent_fp_rate - BLOOM_COUNTER_ERROR_RATE).abs() < 0.001);
}

#[test]
fn test_find_next_missing_transactions() {
    let mut chainstate =
        instantiate_chainstate(false, 0x80000000, "find_next_missing_transactions");
    let chainstate_path = chainstate_path("find_next_missing_transactions");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };

    let block_height = 10;
    let mut txids = vec![];

    let mut mempool_tx = mempool.tx_begin().unwrap();
    for i in 0..(2 * MAX_BLOOM_COUNTER_TXS) {
        let pk = StacksPrivateKey::new();
        let mut tx = StacksTransaction {
            version: TransactionVersion::Testnet,
            chain_id: 0x80000000,
            auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
            anchor_mode: TransactionAnchorMode::Any,
            post_condition_mode: TransactionPostConditionMode::Allow,
            post_conditions: vec![],
            payload: TransactionPayload::TokenTransfer(
                addr.to_account_principal(),
                123,
                TokenTransferMemo([0u8; 34]),
            ),
        };
        tx.set_tx_fee(1000);
        tx.set_origin_nonce(0);

        let txid = tx.txid();
        let tx_bytes = tx.serialize_to_vec();
        let origin_addr = tx.origin_address();
        let origin_nonce = tx.get_origin_nonce();
        let sponsor_addr = tx.sponsor_address().unwrap_or(origin_addr.clone());
        let sponsor_nonce = tx.get_sponsor_nonce().unwrap_or(origin_nonce);
        let tx_fee = tx.get_tx_fee();

        // should succeed
        MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &ConsensusHash([0x1 + (block_height as u8); 20]),
            &BlockHeaderHash([0x2 + (block_height as u8); 32]),
            txid.clone(),
            tx_bytes,
            tx_fee,
            block_height as u64,
            &origin_addr,
            origin_nonce,
            &sponsor_addr,
            sponsor_nonce,
            None,
        )
        .unwrap();

        eprintln!("Added {} {}", i, &txid);
        txids.push(txid);
    }
    mempool_tx.commit().unwrap();

    let mut txid_set = HashSet::new();
    for txid in txids.iter() {
        txid_set.insert(txid.clone());
    }

    eprintln!("Find next missing transactions");

    let txtags = mempool.get_txtags(&[0u8; 32]).unwrap();

    // no txs returned for a full txtag set
    let ts_before = get_epoch_time_ms();
    let (txs, next_page_opt, _) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::TxTags([0u8; 32], txtags.clone()),
            block_height,
            &Txid([0u8; 32]),
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            MAX_BLOOM_COUNTER_TXS as u64,
        )
        .unwrap();
    let ts_after = get_epoch_time_ms();
    eprintln!(
        "find_next_missing_transactions with full txtag set took {} ms",
        ts_after.saturating_sub(ts_before)
    );

    assert_eq!(txs.len(), 0);
    assert!(next_page_opt.is_some());

    // all txs returned for an empty txtag set
    let ts_before = get_epoch_time_ms();
    let (txs, next_page_opt, _) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::TxTags([0u8; 32], vec![]),
            block_height,
            &Txid([0u8; 32]),
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            MAX_BLOOM_COUNTER_TXS as u64,
        )
        .unwrap();
    let ts_after = get_epoch_time_ms();
    eprintln!(
        "find_next_missing_transactions with empty txtag set took {} ms",
        ts_after.saturating_sub(ts_before)
    );

    for tx in txs {
        assert!(txid_set.contains(&tx.txid()));
    }
    assert!(next_page_opt.is_some());

    // all bloom-filter-absent txids should be returned
    let ts_before = get_epoch_time_ms();
    let txid_bloom = mempool.get_txid_bloom_filter().unwrap();
    let (txs, next_page_opt, _) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::BloomFilter(txid_bloom),
            block_height,
            &Txid([0u8; 32]),
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
        )
        .unwrap();
    let ts_after = get_epoch_time_ms();
    eprintln!(
        "find_next_missing_transactions with full bloom filter set took {} ms",
        ts_after.saturating_sub(ts_before)
    );

    assert_eq!(txs.len(), 0);
    assert!(next_page_opt.is_some());

    let mut empty_bloom_conn = setup_bloom_counter("find_next_missing_txs_empty");
    let mut empty_tx = tx_begin_immediate(&mut empty_bloom_conn).unwrap();
    let hasher = BloomNodeHasher::new(&[0u8; 32]);
    let empty_bloom = BloomCounter::new(
        &mut empty_tx,
        "bloom_counter",
        BLOOM_COUNTER_ERROR_RATE,
        MAX_BLOOM_COUNTER_TXS,
        hasher,
    )
    .unwrap();
    empty_tx.commit().unwrap();

    let ts_before = get_epoch_time_ms();
    let (txs, next_page_opt, _) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::BloomFilter(empty_bloom.to_bloom_filter(&empty_bloom_conn).unwrap()),
            block_height,
            &Txid([0u8; 32]),
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
        )
        .unwrap();
    let ts_after = get_epoch_time_ms();
    eprintln!(
        "find_next_missing_transactions with empty bloom filter set took {} ms",
        ts_after.saturating_sub(ts_before)
    );

    for tx in txs {
        assert!(txid_set.contains(&tx.txid()));
    }
    assert!(next_page_opt.is_some());

    // paginated access works too
    let mut last_txid = Txid([0u8; 32]);
    let page_size = 128;
    let mut all_txs = vec![];
    for i in 0..(txtags.len() / (page_size as usize)) + 1 {
        let (mut txs, next_page_opt, num_visited) = mempool
            .find_next_missing_transactions(
                &MemPoolSyncData::TxTags([0u8; 32], vec![]),
                block_height,
                &last_txid,
                (2 * MAX_BLOOM_COUNTER_TXS) as u64,
                page_size,
            )
            .unwrap();
        assert!(txs.len() <= page_size as usize);
        assert!(num_visited <= page_size as u64);

        if txs.len() == 0 {
            assert!(next_page_opt.is_none());
            break;
        }

        last_txid = mempool
            .get_randomized_txid(&txs.last().clone().unwrap().txid())
            .unwrap()
            .unwrap();

        assert_eq!(last_txid, next_page_opt.unwrap());
        all_txs.append(&mut txs);
    }

    for tx in all_txs {
        assert!(txid_set.contains(&tx.txid()));
    }

    last_txid = Txid([0u8; 32]);
    all_txs = vec![];
    for i in 0..(txtags.len() / (page_size as usize)) + 1 {
        let ts_before = get_epoch_time_ms();
        let (mut txs, next_page_opt, num_visited) = mempool
            .find_next_missing_transactions(
                &MemPoolSyncData::BloomFilter(
                    empty_bloom.to_bloom_filter(&empty_bloom_conn).unwrap(),
                ),
                block_height,
                &last_txid,
                (2 * MAX_BLOOM_COUNTER_TXS) as u64,
                page_size,
            )
            .unwrap();
        let ts_after = get_epoch_time_ms();
        eprintln!("find_next_missing_transactions with empty bloom filter took {} ms to serve {} transactions", ts_after.saturating_sub(ts_before), page_size);

        assert!(txs.len() <= page_size as usize);
        assert!(num_visited <= page_size as u64);

        if txs.len() == 0 {
            assert!(next_page_opt.is_none());
            break;
        }

        last_txid = mempool
            .get_randomized_txid(&txs.last().clone().unwrap().txid())
            .unwrap()
            .unwrap();

        assert_eq!(last_txid, next_page_opt.unwrap());
        all_txs.append(&mut txs);
    }

    for tx in all_txs {
        assert!(txid_set.contains(&tx.txid()));
    }

    // old transactions are ignored
    let (old_txs, next_page_opt, num_visited) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::TxTags([0u8; 32], vec![]),
            block_height + (BLOOM_COUNTER_DEPTH as u64) + 1,
            &last_txid,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
        )
        .unwrap();
    assert_eq!(old_txs.len(), 0);
    assert!(next_page_opt.is_none());

    let (old_txs, next_page_opt, num_visited) = mempool
        .find_next_missing_transactions(
            &MemPoolSyncData::BloomFilter(empty_bloom.to_bloom_filter(&empty_bloom_conn).unwrap()),
            block_height + (BLOOM_COUNTER_DEPTH as u64) + 1,
            &last_txid,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
            (2 * MAX_BLOOM_COUNTER_TXS) as u64,
        )
        .unwrap();
    assert_eq!(old_txs.len(), 0);
    assert!(next_page_opt.is_none());
}

#[test]
fn test_stream_txs() {
    let mut chainstate = instantiate_chainstate(false, 0x80000000, "test_stream_txs");
    let chainstate_path = chainstate_path("test_stream_txs");
    let mut mempool = MemPoolDB::open_test(false, 0x80000000, &chainstate_path).unwrap();

    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };
    let mut txs = vec![];
    let block_height = 10;
    let mut total_len = 0;

    let mut mempool_tx = mempool.tx_begin().unwrap();
    for i in 0..10 {
        let pk = StacksPrivateKey::new();
        let mut tx = StacksTransaction {
            version: TransactionVersion::Testnet,
            chain_id: 0x80000000,
            auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
            anchor_mode: TransactionAnchorMode::Any,
            post_condition_mode: TransactionPostConditionMode::Allow,
            post_conditions: vec![],
            payload: TransactionPayload::TokenTransfer(
                addr.to_account_principal(),
                123,
                TokenTransferMemo([0u8; 34]),
            ),
        };
        tx.set_tx_fee(1000);
        tx.set_origin_nonce(0);

        let txid = tx.txid();
        let tx_bytes = tx.serialize_to_vec();
        let origin_addr = tx.origin_address();
        let origin_nonce = tx.get_origin_nonce();
        let sponsor_addr = tx.sponsor_address().unwrap_or(origin_addr.clone());
        let sponsor_nonce = tx.get_sponsor_nonce().unwrap_or(origin_nonce);
        let tx_fee = tx.get_tx_fee();

        total_len += tx_bytes.len();

        // should succeed
        MemPoolDB::try_add_tx(
            &mut mempool_tx,
            &mut chainstate,
            &ConsensusHash([0x1 + (block_height as u8); 20]),
            &BlockHeaderHash([0x2 + (block_height as u8); 32]),
            txid.clone(),
            tx_bytes,
            tx_fee,
            block_height as u64,
            &origin_addr,
            origin_nonce,
            &sponsor_addr,
            sponsor_nonce,
            None,
        )
        .unwrap();

        eprintln!("Added {} {}", i, &txid);
        txs.push(tx);
    }
    mempool_tx.commit().unwrap();

    let mut buf = vec![];
    let stream = StreamCursor::new_tx_stream(
        MemPoolSyncData::TxTags([0u8; 32], vec![]),
        MAX_BLOOM_COUNTER_TXS.into(),
        block_height,
        Some(Txid([0u8; 32])),
    );
    let mut tx_stream_data = if let StreamCursor::MempoolTxs(stream_data) = stream {
        stream_data
    } else {
        unreachable!();
    };

    loop {
        let nw = match mempool.stream_txs(&mut buf, &mut tx_stream_data, 10) {
            Ok(nw) => nw,
            Err(e) => {
                error!("Failed to stream_to: {:?}", &e);
                panic!();
            }
        };
        if nw == 0 {
            break;
        }
    }

    eprintln!("Read {} bytes of tx data", buf.len());

    // buf decodes to the list of txs we have
    let mut decoded_txs = vec![];
    let mut ptr = &buf[..];
    loop {
        let tx: StacksTransaction = match read_next::<StacksTransaction, _>(&mut ptr) {
            Ok(tx) => tx,
            Err(e) => match e {
                codec_error::ReadError(ref ioe) => match ioe.kind() {
                    io::ErrorKind::UnexpectedEof => {
                        eprintln!("out of transactions");
                        break;
                    }
                    _ => {
                        panic!("IO error: {:?}", &e);
                    }
                },
                _ => {
                    panic!("other error: {:?}", &e);
                }
            },
        };
        decoded_txs.push(tx);
    }

    let mut tx_set = HashSet::new();
    for tx in txs.iter() {
        tx_set.insert(tx.txid());
    }

    // the order won't be preserved
    assert_eq!(tx_set.len(), decoded_txs.len());
    for tx in decoded_txs {
        assert!(tx_set.contains(&tx.txid()));
    }

    // verify that we can stream through pagination, with an empty tx tags
    let mut page_id = Txid([0u8; 32]);
    let mut decoded_txs = vec![];
    loop {
        let stream = StreamCursor::new_tx_stream(
            MemPoolSyncData::TxTags([0u8; 32], vec![]),
            1,
            block_height,
            Some(page_id),
        );

        let mut tx_stream_data = if let StreamCursor::MempoolTxs(stream_data) = stream {
            stream_data
        } else {
            unreachable!();
        };

        let mut buf = vec![];
        loop {
            let nw = match mempool.stream_txs(&mut buf, &mut tx_stream_data, 10) {
                Ok(nw) => nw,
                Err(e) => {
                    error!("Failed to stream_to: {:?}", &e);
                    panic!();
                }
            };
            if nw == 0 {
                break;
            }
        }

        // buf decodes to the list of txs we have, plus page ids
        let mut ptr = &buf[..];
        test_debug!("Decode {}", to_hex(ptr));
        let (mut next_txs, next_page) = HttpResponseType::decode_tx_stream(&mut ptr, None).unwrap();

        decoded_txs.append(&mut next_txs);

        // for fun, use a page ID that is actually a well-formed prefix of a transaction
        if let Some(ref tx) = decoded_txs.last() {
            let mut evil_buf = tx.serialize_to_vec();
            let mut evil_page_id = [0u8; 32];
            evil_page_id.copy_from_slice(&evil_buf[0..32]);
            evil_buf.extend_from_slice(&evil_page_id);

            test_debug!("Decode evil buf {}", &to_hex(&evil_buf));

            let (evil_next_txs, evil_next_page) =
                HttpResponseType::decode_tx_stream(&mut &evil_buf[..], None).unwrap();

            // should still work
            assert_eq!(evil_next_txs.len(), 1);
            assert_eq!(evil_next_txs[0].txid(), tx.txid());
            assert_eq!(evil_next_page.unwrap().0[0..32], evil_buf[0..32]);
        }

        if let Some(next_page) = next_page {
            page_id = next_page;
        } else {
            break;
        }
    }

    // make sure we got them all
    let mut tx_set = HashSet::new();
    for tx in txs.iter() {
        tx_set.insert(tx.txid());
    }

    // the order won't be preserved
    assert_eq!(tx_set.len(), decoded_txs.len());
    for tx in decoded_txs {
        assert!(tx_set.contains(&tx.txid()));
    }

    // verify that we can stream through pagination, with a full bloom filter
    let mut page_id = Txid([0u8; 32]);
    let all_txs_tags: Vec<_> = txs
        .iter()
        .map(|tx| TxTag::from(&[0u8; 32], &tx.txid()))
        .collect();
    loop {
        let stream = StreamCursor::new_tx_stream(
            MemPoolSyncData::TxTags([0u8; 32], all_txs_tags.clone()),
            1,
            block_height,
            Some(page_id),
        );

        let mut tx_stream_data = if let StreamCursor::MempoolTxs(stream_data) = stream {
            stream_data
        } else {
            unreachable!();
        };

        let mut buf = vec![];
        loop {
            let nw = match mempool.stream_txs(&mut buf, &mut tx_stream_data, 10) {
                Ok(nw) => nw,
                Err(e) => {
                    error!("Failed to stream_to: {:?}", &e);
                    panic!();
                }
            };
            if nw == 0 {
                break;
            }
        }

        // buf decodes to an empty list of txs, plus page ID
        let mut ptr = &buf[..];
        test_debug!("Decode {}", to_hex(ptr));
        let (next_txs, next_page) = HttpResponseType::decode_tx_stream(&mut ptr, None).unwrap();

        assert_eq!(next_txs.len(), 0);

        if let Some(next_page) = next_page {
            page_id = next_page;
        } else {
            break;
        }
    }
}

#[test]
fn test_decode_tx_stream() {
    let addr = StacksAddress {
        version: 1,
        bytes: Hash160([0xff; 20]),
    };
    let mut txs = vec![];
    for _i in 0..10 {
        let pk = StacksPrivateKey::new();
        let mut tx = StacksTransaction {
            version: TransactionVersion::Testnet,
            chain_id: 0x80000000,
            auth: TransactionAuth::from_p2pkh(&pk).unwrap(),
            anchor_mode: TransactionAnchorMode::Any,
            post_condition_mode: TransactionPostConditionMode::Allow,
            post_conditions: vec![],
            payload: TransactionPayload::TokenTransfer(
                addr.to_account_principal(),
                123,
                TokenTransferMemo([0u8; 34]),
            ),
        };
        tx.set_tx_fee(1000);
        tx.set_origin_nonce(0);
        txs.push(tx);
    }

    // valid empty tx stream
    let empty_stream = [0x11u8; 32];
    let (next_txs, next_page) =
        HttpResponseType::decode_tx_stream(&mut empty_stream.as_ref(), None).unwrap();
    assert_eq!(next_txs.len(), 0);
    assert_eq!(next_page, Some(Txid([0x11; 32])));

    // valid tx stream with a page id at the end
    let mut tx_stream: Vec<u8> = vec![];
    for tx in txs.iter() {
        tx.consensus_serialize(&mut tx_stream).unwrap();
    }
    tx_stream.extend_from_slice(&[0x22; 32]);

    let (next_txs, next_page) =
        HttpResponseType::decode_tx_stream(&mut &tx_stream[..], None).unwrap();
    assert_eq!(next_txs, txs);
    assert_eq!(next_page, Some(Txid([0x22; 32])));

    // valid tx stream with _no_ page id at the end
    let mut partial_stream: Vec<u8> = vec![];
    txs[0].consensus_serialize(&mut partial_stream).unwrap();
    let (next_txs, next_page) =
        HttpResponseType::decode_tx_stream(&mut &partial_stream[..], None).unwrap();
    assert_eq!(next_txs.len(), 1);
    assert_eq!(next_txs[0], txs[0]);
    assert!(next_page.is_none());

    // garbage tx stream
    let garbage_stream = [0xff; 256];
    let err = HttpResponseType::decode_tx_stream(&mut garbage_stream.as_ref(), None);
    match err {
        Err(NetError::ExpectedEndOfStream) => {}
        x => {
            error!("did not fail: {:?}", &x);
            panic!();
        }
    }

    // tx stream that is too short
    let short_stream = [0x33u8; 33];
    let err = HttpResponseType::decode_tx_stream(&mut short_stream.as_ref(), None);
    match err {
        Err(NetError::ExpectedEndOfStream) => {}
        x => {
            error!("did not fail: {:?}", &x);
            panic!();
        }
    }

    // tx stream has a tx, a page ID, and then another tx
    let mut interrupted_stream = vec![];
    txs[0].consensus_serialize(&mut interrupted_stream).unwrap();
    interrupted_stream.extend_from_slice(&[0x00u8; 32]);
    txs[1].consensus_serialize(&mut interrupted_stream).unwrap();

    let err = HttpResponseType::decode_tx_stream(&mut &interrupted_stream[..], None);
    match err {
        Err(NetError::ExpectedEndOfStream) => {}
        x => {
            error!("did not fail: {:?}", &x);
            panic!();
        }
    }
}
