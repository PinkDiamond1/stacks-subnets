// Copyright (C) 2013-2020 Blockstack PBC, a public benefit corporation
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

use crate::codec::StacksMessageCodec;
use crate::types::chainstate::StacksAddress;
use crate::vm::analysis::ContractAnalysis;
use crate::vm::costs::ExecutionCost;
use crate::vm::types::{
    AssetIdentifier, BuffData, PrincipalData, QualifiedContractIdentifier, StandardPrincipalData,
    Value,
};

#[derive(Debug, Clone, PartialEq)]
pub enum StacksTransactionEvent {
    SmartContractEvent(SmartContractEventData),
    STXEvent(STXEventType),
    NFTEvent(NFTEventType),
    FTEvent(FTEventType),
}

impl StacksTransactionEvent {
    pub fn json_serialize(
        &self,
        event_index: usize,
        txid: &dyn std::fmt::Debug,
        committed: bool,
    ) -> serde_json::Value {
        match self {
            StacksTransactionEvent::SmartContractEvent(event_data) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "contract_event",
                "contract_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::STXEvent(STXEventType::STXTransferEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "stx_transfer_event",
                "stx_transfer_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::STXEvent(STXEventType::STXMintEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "stx_mint_event",
                "stx_mint_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::STXEvent(STXEventType::STXBurnEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "stx_burn_event",
                "stx_burn_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::STXEvent(STXEventType::STXLockEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "stx_lock_event",
                "stx_lock_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::STXEvent(STXEventType::STXWithdrawEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "stx_withdraw_event",
                "stx_withdraw_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::NFTEvent(NFTEventType::NFTTransferEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "nft_transfer_event",
                "nft_transfer_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::NFTEvent(NFTEventType::NFTMintEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "nft_mint_event",
                "nft_mint_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::NFTEvent(NFTEventType::NFTBurnEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "nft_burn_event",
                "nft_burn_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::NFTEvent(NFTEventType::NFTWithdrawEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "nft_withdraw_event",
                "nft_withdraw_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::FTEvent(FTEventType::FTTransferEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "ft_transfer_event",
                "ft_transfer_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::FTEvent(FTEventType::FTMintEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "ft_mint_event",
                "ft_mint_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::FTEvent(FTEventType::FTBurnEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "ft_burn_event",
                "ft_burn_event": event_data.json_serialize()
            }),
            StacksTransactionEvent::FTEvent(FTEventType::FTWithdrawEvent(event_data)) => json!({
                "txid": format!("0x{:?}", txid),
                "event_index": event_index,
                "committed": committed,
                "type": "ft_withdraw_event",
                "ft_withdraw_event": event_data.json_serialize()
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum STXEventType {
    STXTransferEvent(STXTransferEventData),
    STXMintEvent(STXMintEventData),
    STXBurnEvent(STXBurnEventData),
    STXLockEvent(STXLockEventData),
    STXWithdrawEvent(STXWithdrawEventData),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NFTEventType {
    NFTTransferEvent(NFTTransferEventData),
    NFTMintEvent(NFTMintEventData),
    NFTBurnEvent(NFTBurnEventData),
    NFTWithdrawEvent(NFTWithdrawEventData),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FTEventType {
    FTTransferEvent(FTTransferEventData),
    FTMintEvent(FTMintEventData),
    FTBurnEvent(FTBurnEventData),
    FTWithdrawEvent(FTWithdrawEventData),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct STXTransferEventData {
    pub sender: PrincipalData,
    pub recipient: PrincipalData,
    pub amount: u128,
}

impl STXTransferEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "sender": format!("{}",self.sender),
            "recipient": format!("{}",self.recipient),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct STXMintEventData {
    pub recipient: PrincipalData,
    pub amount: u128,
}

impl STXMintEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "recipient": format!("{}",self.recipient),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct STXLockEventData {
    pub locked_amount: u128,
    pub unlock_height: u64,
    pub locked_address: PrincipalData,
}

impl STXLockEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "locked_amount": format!("{}",self.locked_amount),
            "unlock_height": format!("{}", self.unlock_height),
            "locked_address": format!("{}", self.locked_address),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct STXBurnEventData {
    pub sender: PrincipalData,
    pub amount: u128,
}

impl STXBurnEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "sender": format!("{}", self.sender),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct STXWithdrawEventData {
    pub sender: PrincipalData,
    pub amount: u128,
    pub withdrawal_id: Option<u32>,
}

impl STXWithdrawEventData {
    /// Serialize to a JSON value. This method fails to serialize if
    /// `withdrawal_id` is not set, returning `None`
    pub fn json_serialize(&self) -> Option<serde_json::Value> {
        Some(json!({
            "sender": self.sender.to_string(),
            "amount": self.amount.to_string(),
            "withdrawal_id": self.withdrawal_id?,
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFTTransferEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub recipient: PrincipalData,
    pub value: Value,
}

impl NFTTransferEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        let raw_value = {
            let mut bytes = vec![];
            self.value.consensus_serialize(&mut bytes).unwrap();
            let formatted_bytes: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            formatted_bytes
        };
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "recipient": format!("{}",self.recipient),
            "value": self.value,
            "raw_value": format!("0x{}", raw_value.join("")),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFTMintEventData {
    pub asset_identifier: AssetIdentifier,
    pub recipient: PrincipalData,
    pub value: Value,
}

impl NFTMintEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        let raw_value = {
            let mut bytes = vec![];
            self.value.consensus_serialize(&mut bytes).unwrap();
            let formatted_bytes: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            formatted_bytes
        };
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "recipient": format!("{}",self.recipient),
            "value": self.value,
            "raw_value": format!("0x{}", raw_value.join("")),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFTBurnEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub value: Value,
}

impl NFTBurnEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        let raw_value = {
            let mut bytes = vec![];
            self.value.consensus_serialize(&mut bytes).unwrap();
            let formatted_bytes: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            formatted_bytes
        };
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "value": self.value,
            "raw_value": format!("0x{}", raw_value.join("")),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFTWithdrawEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub id: u128,
    pub withdrawal_id: Option<u32>,
}

impl NFTWithdrawEventData {
    /// Serialize to a JSON value. This method fails to serialize if
    /// `withdrawal_id` is not set, returning `None`
    pub fn json_serialize(&self) -> Option<serde_json::Value> {
        Some(json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "id": self.id,
            "withdrawal_id": self.withdrawal_id?,
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FTTransferEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub recipient: PrincipalData,
    pub amount: u128,
}

impl FTTransferEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "recipient": format!("{}",self.recipient),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FTMintEventData {
    pub asset_identifier: AssetIdentifier,
    pub recipient: PrincipalData,
    pub amount: u128,
}

impl FTMintEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "recipient": format!("{}",self.recipient),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FTBurnEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub amount: u128,
}

impl FTBurnEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "amount": format!("{}", self.amount),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FTWithdrawEventData {
    pub asset_identifier: AssetIdentifier,
    pub sender: PrincipalData,
    pub amount: u128,
    pub withdrawal_id: Option<u32>,
}

impl FTWithdrawEventData {
    /// Serialize to a JSON value. This method fails to serialize if
    /// `withdrawal_id` is not set, returning `None`
    pub fn json_serialize(&self) -> Option<serde_json::Value> {
        Some(json!({
            "asset_identifier": format!("{}", self.asset_identifier),
            "sender": format!("{}",self.sender),
            "amount": format!("{}", self.amount),
            "withdrawal_id": self.withdrawal_id?,
        }))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmartContractEventData {
    pub key: (QualifiedContractIdentifier, String),
    pub value: Value,
}

impl SmartContractEventData {
    pub fn json_serialize(&self) -> serde_json::Value {
        let raw_value = {
            let mut bytes = vec![];
            self.value.consensus_serialize(&mut bytes).unwrap();
            let formatted_bytes: Vec<String> = bytes.iter().map(|b| format!("{:02x}", b)).collect();
            formatted_bytes
        };
        json!({
            "contract_identifier": self.key.0.to_string(),
            "topic": self.key.1,
            "value": self.value,
            "raw_value": format!("0x{}", raw_value.join("")),
        })
    }
}
