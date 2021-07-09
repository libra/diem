// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    move_types::{account_address::AccountAddress, language_storage::TypeTag},
    types::{
        account_config::{xdx_type_tag, xus_tag, XDX_NAME, XUS_NAME},
        chain_id::ChainId,
        transaction::{authenticator::AuthenticationKey, RawTransaction, TransactionPayload},
    },
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub use diem_transaction_builder::stdlib;
use diem_types::transaction::Script;

pub struct TransactionBuilder {
    sender: Option<AccountAddress>,
    sequence_number: Option<u64>,
    payload: TransactionPayload,
    max_gas_amount: u64,
    gas_unit_price: u64,
    gas_currency_code: String,
    expiration_timestamp_secs: u64,
    chain_id: ChainId,
}

impl TransactionBuilder {
    pub fn sender(mut self, sender: AccountAddress) -> Self {
        self.sender = Some(sender);
        self
    }

    pub fn sequence_number(mut self, sequence_number: u64) -> Self {
        self.sequence_number = Some(sequence_number);
        self
    }

    pub fn max_gas_amount(mut self, max_gas_amount: u64) -> Self {
        self.max_gas_amount = max_gas_amount;
        self
    }

    pub fn gas_unit_price(mut self, gas_unit_price: u64) -> Self {
        self.gas_unit_price = gas_unit_price;
        self
    }

    pub fn gas_currency_code<T: Into<String>>(mut self, gas_currency_code: T) -> Self {
        self.gas_currency_code = gas_currency_code.into();
        self
    }

    pub fn chain_id(mut self, chain_id: ChainId) -> Self {
        self.chain_id = chain_id;
        self
    }

    pub fn expiration_timestamp_secs(mut self, expiration_timestamp_secs: u64) -> Self {
        self.expiration_timestamp_secs = expiration_timestamp_secs;
        self
    }

    pub fn build(self) -> RawTransaction {
        RawTransaction::new(
            self.sender.expect("sender must have been set"),
            self.sequence_number
                .expect("sequence number must have been set"),
            self.payload,
            self.max_gas_amount,
            self.gas_unit_price,
            self.gas_currency_code,
            self.expiration_timestamp_secs,
            self.chain_id,
        )
    }
}

#[derive(Clone, Debug)]
pub struct TransactionFactory {
    max_gas_amount: u64,
    gas_unit_price: u64,
    gas_currency: Currency,
    transaction_expiration_time: u64,
    chain_id: ChainId,
    diem_version: u64,
}

impl TransactionFactory {
    pub fn new(chain_id: ChainId) -> Self {
        Self {
            max_gas_amount: 1_000_000,
            gas_unit_price: 0,
            gas_currency: Currency::XUS,
            transaction_expiration_time: 100,
            chain_id,
            diem_version: 0,
        }
    }

    pub fn with_max_gas_amount(mut self, max_gas_amount: u64) -> Self {
        self.max_gas_amount = max_gas_amount;
        self
    }

    pub fn with_gas_unit_price(mut self, gas_unit_price: u64) -> Self {
        self.gas_unit_price = gas_unit_price;
        self
    }

    pub fn with_gas_currency(mut self, gas_currency: Currency) -> Self {
        self.gas_currency = gas_currency;
        self
    }

    pub fn with_transaction_expiration_time(mut self, transaction_expiration_time: u64) -> Self {
        self.transaction_expiration_time = transaction_expiration_time;
        self
    }

    pub fn with_chain_id(mut self, chain_id: ChainId) -> Self {
        self.chain_id = chain_id;
        self
    }

    pub fn with_diem_version(mut self, diem_version: u64) -> Self {
        self.diem_version = diem_version;
        self
    }

    pub fn payload(&self, payload: TransactionPayload) -> TransactionBuilder {
        self.transaction_builder(payload)
    }

    pub fn add_currency_to_account(&self, currency: Currency) -> TransactionBuilder {
        let currency = currency.type_tag();

        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_add_currency_to_account_script_function(
                currency,
            ))
        } else {
            self.script(stdlib::encode_add_currency_to_account_script(currency))
        }
    }

    pub fn add_recovery_rotation_capability(
        &self,
        recovery_address: AccountAddress,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_add_recovery_rotation_capability_script_function(recovery_address),
            )
        } else {
            self.script(stdlib::encode_add_recovery_rotation_capability_script(
                recovery_address,
            ))
        }
    }

    pub fn add_validator_and_reconfigure(
        &self,
        sliding_nonce: u64,
        validator_name: Vec<u8>,
        validator_address: AccountAddress,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_add_validator_and_reconfigure_script_function(
                    sliding_nonce,
                    validator_name,
                    validator_address,
                ),
            )
        } else {
            self.script(stdlib::encode_add_validator_and_reconfigure_script(
                sliding_nonce,
                validator_name,
                validator_address,
            ))
        }
    }

    pub fn burn_txn_fees(&self, coin_type: Currency) -> TransactionBuilder {
        let coin_type = coin_type.type_tag();

        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_burn_txn_fees_script_function(coin_type))
        } else {
            self.script(stdlib::encode_burn_txn_fees_script(coin_type))
        }
    }

    pub fn burn_with_amount(
        &self,
        token: Currency,
        sliding_nonce: u64,
        preburn_address: AccountAddress,
        amount: u64,
    ) -> TransactionBuilder {
        self.payload(stdlib::encode_burn_with_amount_script_function(
            token.type_tag(),
            sliding_nonce,
            preburn_address,
            amount,
        ))
    }

    pub fn cancel_burn_with_amount(
        &self,
        token: Currency,
        preburn_address: AccountAddress,
        amount: u64,
    ) -> TransactionBuilder {
        self.payload(stdlib::encode_cancel_burn_with_amount_script_function(
            token.type_tag(),
            preburn_address,
            amount,
        ))
    }

    pub fn peer_to_peer(
        &self,
        currency: Currency,
        payee: AccountAddress,
        amount: u64,
    ) -> TransactionBuilder {
        self.peer_to_peer_with_metadata(currency, payee, amount, Vec::new(), Vec::new())
    }

    pub fn peer_to_peer_with_metadata(
        &self,
        currency: Currency,
        payee: AccountAddress,
        amount: u64,
        metadata: Vec<u8>,
        metadata_signature: Vec<u8>,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_peer_to_peer_with_metadata_script_function(
                currency.type_tag(),
                payee,
                amount,
                metadata,
                metadata_signature,
            ))
        } else {
            self.script(stdlib::encode_peer_to_peer_with_metadata_script(
                currency.type_tag(),
                payee,
                amount,
                metadata,
                metadata_signature,
            ))
        }
    }

    pub fn preburn(&self, currency: Currency, amount: u64) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_preburn_script_function(
                currency.type_tag(),
                amount,
            ))
        } else {
            self.script(stdlib::encode_preburn_script(currency.type_tag(), amount))
        }
    }

    pub fn create_child_vasp_account(
        &self,
        coin_type: Currency,
        child_auth_key: AuthenticationKey,
        add_all_currencies: bool,
        child_initial_balance: u64,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_create_child_vasp_account_script_function(
                coin_type.type_tag(),
                child_auth_key.derived_address(),
                child_auth_key.prefix().to_vec(),
                add_all_currencies,
                child_initial_balance,
            ))
        } else {
            self.script(stdlib::encode_create_child_vasp_account_script(
                coin_type.type_tag(),
                child_auth_key.derived_address(),
                child_auth_key.prefix().to_vec(),
                add_all_currencies,
                child_initial_balance,
            ))
        }
    }

    pub fn create_designated_dealer(
        &self,
        coin_type: Currency,
        sliding_nonce: u64,
        auth_key: AuthenticationKey,
        human_name: &str,
        add_all_currencies: bool,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_create_designated_dealer_script_function(
                coin_type.type_tag(),
                sliding_nonce,
                auth_key.derived_address(),
                auth_key.prefix().to_vec(),
                human_name.as_bytes().into(),
                add_all_currencies,
            ))
        } else {
            self.script(stdlib::encode_create_designated_dealer_script(
                coin_type.type_tag(),
                sliding_nonce,
                auth_key.derived_address(),
                auth_key.prefix().to_vec(),
                human_name.as_bytes().into(),
                add_all_currencies,
            ))
        }
    }

    pub fn create_parent_vasp_account(
        &self,
        coin_type: Currency,
        sliding_nonce: u64,
        parent_auth_key: AuthenticationKey,
        human_name: &str,
        add_all_currencies: bool,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_create_parent_vasp_account_script_function(
                coin_type.type_tag(),
                sliding_nonce,
                parent_auth_key.derived_address(),
                parent_auth_key.prefix().to_vec(),
                human_name.as_bytes().into(),
                add_all_currencies,
            ))
        } else {
            self.script(stdlib::encode_create_parent_vasp_account_script(
                coin_type.type_tag(),
                sliding_nonce,
                parent_auth_key.derived_address(),
                parent_auth_key.prefix().to_vec(),
                human_name.as_bytes().into(),
                add_all_currencies,
            ))
        }
    }

    pub fn create_recovery_address(&self) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_create_recovery_address_script_function())
        } else {
            self.script(stdlib::encode_create_recovery_address_script())
        }
    }

    pub fn rotate_authentication_key(&self, new_key: AuthenticationKey) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_rotate_authentication_key_script_function(
                new_key.to_vec(),
            ))
        } else {
            self.script(stdlib::encode_rotate_authentication_key_script(
                new_key.to_vec(),
            ))
        }
    }

    pub fn rotate_authentication_key_with_recovery_address(
        &self,
        recovery_address: AccountAddress,
        to_recover: AccountAddress,
        new_key: AuthenticationKey,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_rotate_authentication_key_with_recovery_address_script_function(
                    recovery_address,
                    to_recover,
                    new_key.to_vec(),
                ),
            )
        } else {
            self.script(
                stdlib::encode_rotate_authentication_key_with_recovery_address_script(
                    recovery_address,
                    to_recover,
                    new_key.to_vec(),
                ),
            )
        }
    }

    pub fn rotate_dual_attestation_info(
        &self,
        new_url: Vec<u8>,
        new_key: Vec<u8>,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_rotate_dual_attestation_info_script_function(
                new_url, new_key,
            ))
        } else {
            self.script(stdlib::encode_rotate_dual_attestation_info_script(
                new_url, new_key,
            ))
        }
    }

    pub fn publish_shared_ed25519_public_key(&self, public_key: Vec<u8>) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_publish_shared_ed25519_public_key_script_function(public_key),
            )
        } else {
            self.script(stdlib::encode_publish_shared_ed25519_public_key_script(
                public_key,
            ))
        }
    }

    pub fn publish_rotate_ed25519_public_key(&self, public_key: Vec<u8>) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_rotate_shared_ed25519_public_key_script_function(public_key),
            )
        } else {
            self.script(stdlib::encode_rotate_shared_ed25519_public_key_script(
                public_key,
            ))
        }
    }

    pub fn update_diem_version(&self, sliding_nonce: u64, major: u64) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_update_diem_version_script_function(
                sliding_nonce,
                major,
            ))
        } else {
            self.script(stdlib::encode_update_diem_version_script(
                sliding_nonce,
                major,
            ))
        }
    }

    pub fn update_exchange_rate(
        &self,
        currency: Currency,
        sliding_nonce: u64,
        exchange_rate_numerator: u64,
        exchange_rate_denominator: u64,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(stdlib::encode_update_exchange_rate_script_function(
                currency.type_tag(),
                sliding_nonce,
                exchange_rate_numerator,
                exchange_rate_denominator,
            ))
        } else {
            self.script(stdlib::encode_update_exchange_rate_script(
                currency.type_tag(),
                sliding_nonce,
                exchange_rate_numerator,
                exchange_rate_denominator,
            ))
        }
    }

    pub fn remove_validator_and_reconfigure(
        &self,
        sliding_nonce: u64,
        validator_name: Vec<u8>,
        validator_address: AccountAddress,
    ) -> TransactionBuilder {
        if self.is_script_function_enabled() {
            self.payload(
                stdlib::encode_remove_validator_and_reconfigure_script_function(
                    sliding_nonce,
                    validator_name,
                    validator_address,
                ),
            )
        } else {
            self.script(stdlib::encode_remove_validator_and_reconfigure_script(
                sliding_nonce,
                validator_name,
                validator_address,
            ))
        }
    }

    pub fn add_vasp_domain(&self, address: AccountAddress, domain: Vec<u8>) -> TransactionBuilder {
        self.payload(stdlib::encode_add_vasp_domain_script_function(
            address, domain,
        ))
    }

    pub fn remove_vasp_domain(
        &self,
        address: AccountAddress,
        domain: Vec<u8>,
    ) -> TransactionBuilder {
        self.payload(stdlib::encode_remove_vasp_domain_script_function(
            address, domain,
        ))
    }

    //
    // Internal Helpers
    //

    pub fn script(&self, script: Script) -> TransactionBuilder {
        self.payload(TransactionPayload::Script(script))
    }

    fn transaction_builder(&self, payload: TransactionPayload) -> TransactionBuilder {
        TransactionBuilder {
            sender: None,
            sequence_number: None,
            payload,
            max_gas_amount: self.max_gas_amount,
            gas_unit_price: self.gas_unit_price,
            gas_currency_code: self.gas_currency.into(),
            expiration_timestamp_secs: self.expiration_timestamp(),
            chain_id: self.chain_id,
        }
    }

    fn expiration_timestamp(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + self.transaction_expiration_time
    }

    fn is_script_function_enabled(&self) -> bool {
        self.diem_version >= 2
    }
}

pub struct DualAttestationMessage {
    message: Box<[u8]>,
}

impl DualAttestationMessage {
    pub fn new<M: Into<Vec<u8>>>(metadata: M, reciever: AccountAddress, amount: u64) -> Self {
        let mut message = metadata.into();
        bcs::serialize_into(&mut message, &reciever).unwrap();
        bcs::serialize_into(&mut message, &amount).unwrap();
        message.extend(b"@@$$DIEM_ATTEST$$@@");

        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &[u8] {
        &self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    XDX,
    XUS,
}

impl Currency {
    pub fn as_str(&self) -> &str {
        match self {
            Currency::XDX => XDX_NAME,
            Currency::XUS => XUS_NAME,
        }
    }

    pub fn type_tag(&self) -> TypeTag {
        match self {
            Currency::XDX => xdx_type_tag(),
            Currency::XUS => xus_tag(),
        }
    }
}

impl PartialEq<str> for Currency {
    fn eq(&self, other: &str) -> bool {
        self.as_str().eq(other)
    }
}

impl PartialEq<Currency> for str {
    fn eq(&self, other: &Currency) -> bool {
        other.as_str().eq(self)
    }
}

impl PartialEq<String> for Currency {
    fn eq(&self, other: &String) -> bool {
        self.as_str().eq(other)
    }
}

impl PartialEq<Currency> for String {
    fn eq(&self, other: &Currency) -> bool {
        other.as_str().eq(self)
    }
}

impl From<Currency> for String {
    fn from(currency: Currency) -> Self {
        currency.as_str().to_owned()
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
