// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    account_address::AccountAddress,
    account_config::XUS_NAME,
    account_state_blob::AccountStateBlob,
    block_metadata::BlockMetadata,
    chain_id::ChainId,
    contract_event::ContractEvent,
    ledger_info::LedgerInfo,
    nibble::nibble_path::NibblePath,
    proof::{accumulator::InMemoryAccumulator, TransactionInfoWithProof, TransactionListProof},
    transaction::authenticator::{AccountAuthenticator, TransactionAuthenticator},
    vm_status::{DiscardedVMStatus, KeptVMStatus, StatusCode, StatusType, VMStatus},
    write_set::WriteSet,
};
use anyhow::{ensure, format_err, Error, Result};
use diem_crypto::{
    ed25519::*,
    hash::{CryptoHash, EventAccumulatorHasher},
    multi_ed25519::{MultiEd25519PublicKey, MultiEd25519Signature},
    traits::SigningKey,
    HashValue,
};
use diem_crypto_derive::{BCSCryptoHash, CryptoHasher};
use move_core_types::transaction_argument::convert_txn_args;
#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::TryFrom,
    fmt,
    fmt::{Display, Formatter},
};

pub mod authenticator;
mod change_set;
pub mod helpers;
pub mod metadata;
mod module;
mod script;
mod transaction_argument;

pub use change_set::ChangeSet;
pub use module::Module;
pub use script::{
    ArgumentABI, Script, ScriptABI, ScriptFunction, ScriptFunctionABI, TransactionScriptABI,
    TypeArgumentABI,
};

use std::{collections::BTreeSet, ops::Deref};
pub use transaction_argument::{parse_transaction_argument, TransactionArgument};

pub type Version = u64; // Height - also used for MVCC in StateDB

// In StateDB, things readable by the genesis transaction are under this version.
pub const PRE_GENESIS_VERSION: Version = u64::max_value();

/// RawTransaction is the portion of a transaction that a client signs.
#[derive(
    Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize, CryptoHasher, BCSCryptoHash,
)]
pub struct RawTransaction {
    /// Sender's address.
    sender: AccountAddress,

    /// Sequence number of this transaction. This must match the sequence number
    /// stored in the sender's account at the time the transaction executes.
    sequence_number: u64,

    /// The transaction payload, e.g., a script to execute.
    payload: TransactionPayload,

    /// Maximal total gas to spend for this transaction.
    max_gas_amount: u64,

    /// Price to be paid per gas unit.
    gas_unit_price: u64,

    /// The currency code, e.g., "XUS", used to pay for gas. The `max_gas_amount`
    /// and `gas_unit_price` values refer to units of this currency.
    gas_currency_code: String,

    /// Expiration timestamp for this transaction, represented
    /// as seconds from the Unix Epoch. If the current blockchain timestamp
    /// is greater than or equal to this time, then the transaction has
    /// expired and will be discarded. This can be set to a large value far
    /// in the future to indicate that a transaction does not expire.
    expiration_timestamp_secs: u64,

    /// Chain ID of the Diem network this transaction is intended for.
    chain_id: ChainId,
}

impl RawTransaction {
    /// Create a new `RawTransaction` with a payload.
    ///
    /// It can be either to publish a module, to execute a script, or to issue a writeset
    /// transaction.
    pub fn new(
        sender: AccountAddress,
        sequence_number: u64,
        payload: TransactionPayload,
        max_gas_amount: u64,
        gas_unit_price: u64,
        gas_currency_code: String,
        expiration_timestamp_secs: u64,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload,
            max_gas_amount,
            gas_unit_price,
            gas_currency_code,
            expiration_timestamp_secs,
            chain_id,
        }
    }

    /// Create a new `RawTransaction` with a script.
    ///
    /// A script transaction contains only code to execute. No publishing is allowed in scripts.
    pub fn new_script(
        sender: AccountAddress,
        sequence_number: u64,
        script: Script,
        max_gas_amount: u64,
        gas_unit_price: u64,
        gas_currency_code: String,
        expiration_timestamp_secs: u64,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload: TransactionPayload::Script(script),
            max_gas_amount,
            gas_unit_price,
            gas_currency_code,
            expiration_timestamp_secs,
            chain_id,
        }
    }

    /// Create a new `RawTransaction` with a script function.
    ///
    /// A script transaction contains only code to execute. No publishing is allowed in scripts.
    pub fn new_script_function(
        sender: AccountAddress,
        sequence_number: u64,
        script_function: ScriptFunction,
        max_gas_amount: u64,
        gas_unit_price: u64,
        gas_currency_code: String,
        expiration_timestamp_secs: u64,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload: TransactionPayload::ScriptFunction(script_function),
            max_gas_amount,
            gas_unit_price,
            gas_currency_code,
            expiration_timestamp_secs,
            chain_id,
        }
    }

    /// Create a new `RawTransaction` with a module to publish.
    ///
    /// A module transaction is the only way to publish code. Only one module per transaction
    /// can be published.
    pub fn new_module(
        sender: AccountAddress,
        sequence_number: u64,
        module: Module,
        max_gas_amount: u64,
        gas_unit_price: u64,
        gas_currency_code: String,
        expiration_timestamp_secs: u64,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload: TransactionPayload::Module(module),
            max_gas_amount,
            gas_unit_price,
            gas_currency_code,
            expiration_timestamp_secs,
            chain_id,
        }
    }

    pub fn new_write_set(
        sender: AccountAddress,
        sequence_number: u64,
        write_set: WriteSet,
        chain_id: ChainId,
    ) -> Self {
        Self::new_change_set(
            sender,
            sequence_number,
            ChangeSet::new(write_set, vec![]),
            chain_id,
        )
    }

    pub fn new_change_set(
        sender: AccountAddress,
        sequence_number: u64,
        change_set: ChangeSet,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload: TransactionPayload::WriteSet(WriteSetPayload::Direct(change_set)),
            // Since write-set transactions bypass the VM, these fields aren't relevant.
            max_gas_amount: 0,
            gas_unit_price: 0,
            gas_currency_code: XUS_NAME.to_owned(),
            // Write-set transactions are special and important and shouldn't expire.
            expiration_timestamp_secs: u64::max_value(),
            chain_id,
        }
    }

    pub fn new_writeset_script(
        sender: AccountAddress,
        sequence_number: u64,
        script: Script,
        signer: AccountAddress,
        chain_id: ChainId,
    ) -> Self {
        RawTransaction {
            sender,
            sequence_number,
            payload: TransactionPayload::WriteSet(WriteSetPayload::Script {
                execute_as: signer,
                script,
            }),
            // Since write-set transactions bypass the VM, these fields aren't relevant.
            max_gas_amount: 0,
            gas_unit_price: 0,
            gas_currency_code: XUS_NAME.to_owned(),
            // Write-set transactions are special and important and shouldn't expire.
            expiration_timestamp_secs: u64::max_value(),
            chain_id,
        }
    }

    /// Signs the given `RawTransaction`. Note that this consumes the `RawTransaction` and turns it
    /// into a `SignatureCheckedTransaction`.
    ///
    /// For a transaction that has just been signed, its signature is expected to be valid.
    pub fn sign(
        self,
        private_key: &Ed25519PrivateKey,
        public_key: Ed25519PublicKey,
    ) -> Result<SignatureCheckedTransaction> {
        let signature = private_key.sign(&self);
        Ok(SignatureCheckedTransaction(SignedTransaction::new(
            self, public_key, signature,
        )))
    }

    /// Signs the given multi-agent `RawTransaction`, which is a transaction with secondary
    /// signers in addition to a sender. The private keys of the sender and the
    /// secondary signers are used to sign the transaction.
    ///
    /// The order and length of the secondary keys provided here have to match the order and
    /// length of the `secondary_signers`.
    pub fn sign_multi_agent(
        self,
        sender_private_key: &Ed25519PrivateKey,
        secondary_signers: Vec<AccountAddress>,
        secondary_private_keys: Vec<&Ed25519PrivateKey>,
    ) -> Result<SignatureCheckedTransaction> {
        let message =
            RawTransactionWithData::new_multi_agent(self.clone(), secondary_signers.clone());
        let sender_signature = sender_private_key.sign(&message);
        let sender_authenticator = AccountAuthenticator::ed25519(
            Ed25519PublicKey::from(sender_private_key),
            sender_signature,
        );

        if secondary_private_keys.len() != secondary_signers.len() {
            return Err(format_err!(
                "number of secondary private keys and number of secondary signers don't match"
            ));
        }
        let mut secondary_authenticators = vec![];
        for priv_key in secondary_private_keys {
            let signature = priv_key.sign(&message);
            secondary_authenticators.push(AccountAuthenticator::ed25519(
                Ed25519PublicKey::from(priv_key),
                signature,
            ));
        }

        Ok(SignatureCheckedTransaction(
            SignedTransaction::new_multi_agent(
                self,
                sender_authenticator,
                secondary_signers,
                secondary_authenticators,
            ),
        ))
    }

    #[cfg(any(test, feature = "fuzzing"))]
    pub fn multi_sign_for_testing(
        self,
        private_key: &Ed25519PrivateKey,
        public_key: Ed25519PublicKey,
    ) -> Result<SignatureCheckedTransaction> {
        let signature = private_key.sign(&self);
        Ok(SignatureCheckedTransaction(
            SignedTransaction::new_multisig(self, public_key.into(), signature.into()),
        ))
    }

    pub fn into_payload(self) -> TransactionPayload {
        self.payload
    }

    pub fn format_for_client(&self, get_transaction_name: impl Fn(&[u8]) -> String) -> String {
        let (code, args) = match &self.payload {
            TransactionPayload::WriteSet(_) => ("genesis".to_string(), vec![]),
            TransactionPayload::Script(script) => (
                get_transaction_name(script.code()),
                convert_txn_args(script.args()),
            ),
            TransactionPayload::ScriptFunction(script_fn) => (
                format!("{}::{}", script_fn.module(), script_fn.function()),
                script_fn.args().to_vec(),
            ),
            TransactionPayload::Module(_) => ("module publishing".to_string(), vec![]),
        };
        let mut f_args: String = "".to_string();
        for arg in args {
            f_args = format!("{}\n\t\t\t{:02X?},", f_args, arg);
        }
        format!(
            "RawTransaction {{ \n\
             \tsender: {}, \n\
             \tsequence_number: {}, \n\
             \tpayload: {{, \n\
             \t\ttransaction: {}, \n\
             \t\targs: [ {} \n\
             \t\t]\n\
             \t}}, \n\
             \tmax_gas_amount: {}, \n\
             \tgas_unit_price: {}, \n\
             \tgas_currency_code: {}, \n\
             \texpiration_timestamp_secs: {:#?}, \n\
             \tchain_id: {},
             }}",
            self.sender,
            self.sequence_number,
            code,
            f_args,
            self.max_gas_amount,
            self.gas_unit_price,
            self.gas_currency_code,
            self.expiration_timestamp_secs,
            self.chain_id,
        )
    }
    /// Return the sender of this transaction.
    pub fn sender(&self) -> AccountAddress {
        self.sender
    }
}

#[derive(
    Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize, CryptoHasher, BCSCryptoHash,
)]
pub enum RawTransactionWithData {
    MultiAgent {
        raw_txn: RawTransaction,
        secondary_signer_addresses: Vec<AccountAddress>,
    },
}

impl RawTransactionWithData {
    pub fn new_multi_agent(
        raw_txn: RawTransaction,
        secondary_signer_addresses: Vec<AccountAddress>,
    ) -> Self {
        Self::MultiAgent {
            raw_txn,
            secondary_signer_addresses,
        }
    }
}

/// Different kinds of transactions.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransactionPayload {
    /// A system maintenance transaction.
    WriteSet(WriteSetPayload),
    /// A transaction that executes code.
    Script(Script),
    /// A transaction that publishes code.
    Module(Module),
    /// A transaction that executes an existing script function published on-chain.
    ScriptFunction(ScriptFunction),
}

impl TransactionPayload {
    pub fn should_trigger_reconfiguration_by_default(&self) -> bool {
        match self {
            Self::WriteSet(ws) => ws.should_trigger_reconfiguration_by_default(),
            Self::Script(_) | Self::ScriptFunction(_) | Self::Module(_) => false,
        }
    }

    pub fn into_script_function(self) -> ScriptFunction {
        match self {
            Self::ScriptFunction(f) => f,
            payload => panic!("Expected ScriptFunction(_) payload, found: {:#?}", payload),
        }
    }
}

/// Two different kinds of WriteSet transactions.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum WriteSetPayload {
    /// Directly passing in the WriteSet.
    Direct(ChangeSet),
    /// Generate the WriteSet by running a script.
    Script {
        /// Execute the script as the designated signer.
        execute_as: AccountAddress,
        /// Script body that gets executed.
        script: Script,
    },
}

impl WriteSetPayload {
    pub fn should_trigger_reconfiguration_by_default(&self) -> bool {
        match self {
            Self::Direct(_) => true,
            Self::Script { .. } => false,
        }
    }
}

/// A transaction that has been signed.
///
/// A `SignedTransaction` is a single transaction that can be atomically executed. Clients submit
/// these to validator nodes, and the validator and executor submits these to the VM.
///
/// **IMPORTANT:** The signature of a `SignedTransaction` is not guaranteed to be verified. For a
/// transaction whose signature is statically guaranteed to be verified, see
/// [`SignatureCheckedTransaction`].
#[derive(Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SignedTransaction {
    /// The raw transaction
    raw_txn: RawTransaction,

    /// Public key and signature to authenticate
    authenticator: TransactionAuthenticator,
}

/// A transaction for which the signature has been verified. Created by
/// [`SignedTransaction::check_signature`] and [`RawTransaction::sign`].
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SignatureCheckedTransaction(SignedTransaction);

impl SignatureCheckedTransaction {
    /// Returns the `SignedTransaction` within.
    pub fn into_inner(self) -> SignedTransaction {
        self.0
    }

    /// Returns the `RawTransaction` within.
    pub fn into_raw_transaction(self) -> RawTransaction {
        self.0.into_raw_transaction()
    }
}

impl Deref for SignatureCheckedTransaction {
    type Target = SignedTransaction;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for SignedTransaction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SignedTransaction {{ \n \
             {{ raw_txn: {:#?}, \n \
             authenticator: {:#?}, \n \
             }} \n \
             }}",
            self.raw_txn, self.authenticator
        )
    }
}

impl SignedTransaction {
    pub fn new(
        raw_txn: RawTransaction,
        public_key: Ed25519PublicKey,
        signature: Ed25519Signature,
    ) -> SignedTransaction {
        let authenticator = TransactionAuthenticator::ed25519(public_key, signature);
        SignedTransaction {
            raw_txn,
            authenticator,
        }
    }

    pub fn new_multisig(
        raw_txn: RawTransaction,
        public_key: MultiEd25519PublicKey,
        signature: MultiEd25519Signature,
    ) -> SignedTransaction {
        let authenticator = TransactionAuthenticator::multi_ed25519(public_key, signature);
        SignedTransaction {
            raw_txn,
            authenticator,
        }
    }

    pub fn new_multi_agent(
        raw_txn: RawTransaction,
        sender: AccountAuthenticator,
        secondary_signer_addresses: Vec<AccountAddress>,
        secondary_signers: Vec<AccountAuthenticator>,
    ) -> Self {
        SignedTransaction {
            raw_txn,
            authenticator: TransactionAuthenticator::multi_agent(
                sender,
                secondary_signer_addresses,
                secondary_signers,
            ),
        }
    }

    pub fn authenticator(&self) -> TransactionAuthenticator {
        self.authenticator.clone()
    }

    pub fn sender(&self) -> AccountAddress {
        self.raw_txn.sender
    }

    pub fn into_raw_transaction(self) -> RawTransaction {
        self.raw_txn
    }

    pub fn sequence_number(&self) -> u64 {
        self.raw_txn.sequence_number
    }

    pub fn chain_id(&self) -> ChainId {
        self.raw_txn.chain_id
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.raw_txn.payload
    }

    pub fn max_gas_amount(&self) -> u64 {
        self.raw_txn.max_gas_amount
    }

    pub fn gas_unit_price(&self) -> u64 {
        self.raw_txn.gas_unit_price
    }

    pub fn gas_currency_code(&self) -> &str {
        &self.raw_txn.gas_currency_code
    }

    pub fn expiration_timestamp_secs(&self) -> u64 {
        self.raw_txn.expiration_timestamp_secs
    }

    pub fn raw_txn_bytes_len(&self) -> usize {
        bcs::to_bytes(&self.raw_txn)
            .expect("Unable to serialize RawTransaction")
            .len()
    }

    /// Checks that the signature of given transaction. Returns `Ok(SignatureCheckedTransaction)` if
    /// the signature is valid.
    pub fn check_signature(self) -> Result<SignatureCheckedTransaction> {
        self.authenticator.verify(&self.raw_txn)?;
        Ok(SignatureCheckedTransaction(self))
    }

    pub fn contains_duplicate_signers(&self) -> bool {
        let mut all_signer_addresses = self.authenticator.secondary_signer_addreses();
        all_signer_addresses.push(self.sender());
        let mut s = BTreeSet::new();
        all_signer_addresses.iter().any(|a| !s.insert(*a))
    }

    pub fn format_for_client(&self, get_transaction_name: impl Fn(&[u8]) -> String) -> String {
        format!(
            "SignedTransaction {{ \n \
             raw_txn: {}, \n \
             authenticator: {:#?}, \n \
             }}",
            self.raw_txn.format_for_client(get_transaction_name),
            self.authenticator
        )
    }

    pub fn is_multi_agent(&self) -> bool {
        matches!(
            self.authenticator,
            TransactionAuthenticator::MultiAgent { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct TransactionWithProof {
    pub version: Version,
    pub transaction: Transaction,
    pub events: Option<Vec<ContractEvent>>,
    pub proof: TransactionInfoWithProof,
}

impl TransactionWithProof {
    pub fn new(
        version: Version,
        transaction: Transaction,
        events: Option<Vec<ContractEvent>>,
        proof: TransactionInfoWithProof,
    ) -> Self {
        Self {
            version,
            transaction,
            events,
            proof,
        }
    }
    /// Verifies the transaction with the proof, both carried by `self`.
    ///
    /// A few things are ensured if no error is raised:
    ///   1. This transaction exists in the ledger represented by `ledger_info`.
    ///   2. This transaction is a `UserTransaction`.
    ///   3. And this user transaction has the same `version`, `sender`, and `sequence_number` as
    ///      indicated by the parameter list. If any of these parameter is unknown to the call site
    ///      that is supposed to be informed via this struct, get it from the struct itself, such
    ///      as version and sender.
    pub fn verify_user_txn(
        &self,
        ledger_info: &LedgerInfo,
        version: Version,
        sender: AccountAddress,
        sequence_number: u64,
    ) -> Result<()> {
        let signed_transaction = self.transaction.as_signed_user_txn()?;

        ensure!(
            self.version == version,
            "Version ({}) is not expected ({}).",
            self.version,
            version,
        );
        ensure!(
            signed_transaction.sender() == sender,
            "Sender ({}) not expected ({}).",
            signed_transaction.sender(),
            sender,
        );
        ensure!(
            signed_transaction.sequence_number() == sequence_number,
            "Sequence number ({}) not expected ({}).",
            signed_transaction.sequence_number(),
            sequence_number,
        );

        let txn_hash = self.transaction.hash();
        ensure!(
            txn_hash == self.proof.transaction_info().transaction_hash,
            "Transaction hash ({}) not expected ({}).",
            txn_hash,
            self.proof.transaction_info().transaction_hash,
        );

        if let Some(events) = &self.events {
            let event_hashes: Vec<_> = events.iter().map(ContractEvent::hash).collect();
            let event_root_hash =
                InMemoryAccumulator::<EventAccumulatorHasher>::from_leaves(&event_hashes[..])
                    .root_hash();
            ensure!(
                event_root_hash == self.proof.transaction_info().event_root_hash,
                "Event root hash ({}) not expected ({}).",
                event_root_hash,
                self.proof.transaction_info().event_root_hash,
            );
        }

        self.proof.verify(ledger_info, version)
    }
}

/// The status of executing a transaction. The VM decides whether or not we should `Keep` the
/// transaction output or `Discard` it based upon the execution of the transaction. We wrap these
/// decisions around a `VMStatus` that provides more detail on the final execution state of the VM.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TransactionStatus {
    /// Discard the transaction output
    Discard(DiscardedVMStatus),

    /// Keep the transaction output
    Keep(KeptVMStatus),

    /// Retry the transaction, e.g., after a reconfiguration
    Retry,
}

impl TransactionStatus {
    pub fn status(&self) -> Result<KeptVMStatus, StatusCode> {
        match self {
            TransactionStatus::Keep(status) => Ok(status.clone()),
            TransactionStatus::Discard(code) => Err(*code),
            TransactionStatus::Retry => Err(StatusCode::UNKNOWN_VALIDATION_STATUS),
        }
    }

    pub fn is_discarded(&self) -> bool {
        match self {
            TransactionStatus::Discard(_) => true,
            TransactionStatus::Keep(_) => false,
            TransactionStatus::Retry => true,
        }
    }
}

impl From<VMStatus> for TransactionStatus {
    fn from(vm_status: VMStatus) -> Self {
        match vm_status.keep_or_discard() {
            Ok(recorded) => TransactionStatus::Keep(recorded),
            Err(code) => TransactionStatus::Discard(code),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum GovernanceRole {
    DiemRoot,
    TreasuryCompliance,
    Validator,
    ValidatorOperator,
    DesignatedDealer,
    NonGovernanceRole,
}

impl GovernanceRole {
    pub fn from_role_id(role_id: u64) -> Self {
        use GovernanceRole::*;
        match role_id {
            0 => DiemRoot,
            1 => TreasuryCompliance,
            2 => DesignatedDealer,
            3 => Validator,
            4 => ValidatorOperator,
            _ => NonGovernanceRole,
        }
    }

    /// The higher the number that is returned, the greater priority assigned to a transaction sent
    /// from an account with that role in mempool. All transactions sent from an account with role
    /// priority N are ranked higher than all transactions sent from accounts with role priorities < N.
    /// Transactions from accounts with equal priority are ranked base on other characteristics (e.g., gas price).
    pub fn priority(&self) -> u64 {
        use GovernanceRole::*;
        match self {
            DiemRoot => 3,
            TreasuryCompliance => 2,
            Validator | ValidatorOperator | DesignatedDealer => 1,
            NonGovernanceRole => 0,
        }
    }
}

/// The result of running the transaction through the VM validator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VMValidatorResult {
    /// Result of the validation: `None` if the transaction was successfully validated
    /// or `Some(DiscardedVMStatus)` if the transaction should be discarded.
    status: Option<DiscardedVMStatus>,

    /// Score for ranking the transaction priority (e.g., based on the gas price).
    /// Only used when the status is `None`. Higher values indicate a higher priority.
    score: u64,

    /// The account role for the transaction sender, so that certain
    /// governance transactions can be prioritized above normal transactions.
    /// Only used when the status is `None`.
    governance_role: GovernanceRole,
}

impl VMValidatorResult {
    pub fn new(
        vm_status: Option<DiscardedVMStatus>,
        score: u64,
        governance_role: GovernanceRole,
    ) -> Self {
        debug_assert!(
            match vm_status {
                None => true,
                Some(status) =>
                    status.status_type() == StatusType::Unknown
                        || status.status_type() == StatusType::Validation
                        || status.status_type() == StatusType::InvariantViolation,
            },
            "Unexpected discarded status: {:?}",
            vm_status
        );
        Self {
            status: vm_status,
            score,
            governance_role,
        }
    }

    pub fn error(vm_status: DiscardedVMStatus) -> Self {
        Self {
            status: Some(vm_status),
            score: 0,
            governance_role: GovernanceRole::NonGovernanceRole,
        }
    }

    pub fn status(&self) -> Option<DiscardedVMStatus> {
        self.status
    }

    pub fn score(&self) -> u64 {
        self.score
    }

    pub fn governance_role(&self) -> GovernanceRole {
        self.governance_role
    }
}

/// The output of executing a transaction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransactionOutput {
    /// The list of writes this transaction intends to do.
    write_set: WriteSet,

    /// The list of events emitted during this transaction.
    events: Vec<ContractEvent>,

    /// The amount of gas used during execution.
    gas_used: u64,

    /// The execution status.
    status: TransactionStatus,
}

impl TransactionOutput {
    pub fn new(
        write_set: WriteSet,
        events: Vec<ContractEvent>,
        gas_used: u64,
        status: TransactionStatus,
    ) -> Self {
        TransactionOutput {
            write_set,
            events,
            gas_used,
            status,
        }
    }

    pub fn into(self) -> (WriteSet, Vec<ContractEvent>) {
        (self.write_set, self.events)
    }

    pub fn write_set(&self) -> &WriteSet {
        &self.write_set
    }

    pub fn events(&self) -> &[ContractEvent] {
        &self.events
    }

    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    pub fn status(&self) -> &TransactionStatus {
        &self.status
    }
}

/// `TransactionInfo` is the object we store in the transaction accumulator. It consists of the
/// transaction as well as the execution result of this transaction.
#[derive(Clone, CryptoHasher, BCSCryptoHash, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct TransactionInfo {
    /// The hash of this transaction.
    transaction_hash: HashValue,

    /// The root hash of Sparse Merkle Tree describing the world state at the end of this
    /// transaction.
    state_root_hash: HashValue,

    /// The root hash of Merkle Accumulator storing all events emitted during this transaction.
    event_root_hash: HashValue,

    /// The amount of gas used.
    gas_used: u64,

    /// The vm status. If it is not `Executed`, this will provide the general error class. Execution
    /// failures and Move abort's recieve more detailed information. But other errors are generally
    /// categorized with no status code or other information
    status: KeptVMStatus,
}

impl TransactionInfo {
    /// Constructs a new `TransactionInfo` object using transaction hash, state root hash and event
    /// root hash.
    pub fn new(
        transaction_hash: HashValue,
        state_root_hash: HashValue,
        event_root_hash: HashValue,
        gas_used: u64,
        status: KeptVMStatus,
    ) -> TransactionInfo {
        TransactionInfo {
            transaction_hash,
            state_root_hash,
            event_root_hash,
            gas_used,
            status,
        }
    }

    /// Returns the hash of this transaction.
    pub fn transaction_hash(&self) -> HashValue {
        self.transaction_hash
    }

    /// Returns root hash of Sparse Merkle Tree describing the world state at the end of this
    /// transaction.
    pub fn state_root_hash(&self) -> HashValue {
        self.state_root_hash
    }

    /// Returns the root hash of Merkle Accumulator storing all events emitted during this
    /// transaction.
    pub fn event_root_hash(&self) -> HashValue {
        self.event_root_hash
    }

    /// Returns the amount of gas used by this transaction.
    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    pub fn status(&self) -> &KeptVMStatus {
        &self.status
    }
}

impl Display for TransactionInfo {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(
            f,
            "TransactionInfo: [txn_hash: {}, state_root_hash: {}, event_root_hash: {}, gas_used: {}, recorded_status: {:?}]",
            self.transaction_hash(), self.state_root_hash(), self.event_root_hash(), self.gas_used(), self.status(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TransactionToCommit {
    transaction: Transaction,
    account_states: HashMap<AccountAddress, AccountStateBlob>,
    jf_node_hashes: HashMap<NibblePath, HashValue>,
    events: Vec<ContractEvent>,
    gas_used: u64,
    status: KeptVMStatus,
}

impl TransactionToCommit {
    pub fn new(
        transaction: Transaction,
        account_states: HashMap<AccountAddress, AccountStateBlob>,
        jf_node_hashes: HashMap<NibblePath, HashValue>,
        events: Vec<ContractEvent>,
        gas_used: u64,
        status: KeptVMStatus,
    ) -> Self {
        TransactionToCommit {
            transaction,
            account_states,
            jf_node_hashes,
            events,
            gas_used,
            status,
        }
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn account_states(&self) -> &HashMap<AccountAddress, AccountStateBlob> {
        &self.account_states
    }

    pub fn jf_node_hashes(&self) -> &HashMap<NibblePath, HashValue> {
        &self.jf_node_hashes
    }

    pub fn events(&self) -> &[ContractEvent] {
        &self.events
    }

    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    pub fn status(&self) -> &KeptVMStatus {
        &self.status
    }
}

/// The list may have three states:
/// 1. The list is empty. Both proofs must be `None`.
/// 2. The list has only 1 transaction/transaction_info. Then `proof_of_first_transaction`
/// must exist and `proof_of_last_transaction` must be `None`.
/// 3. The list has 2+ transactions/transaction_infos. The both proofs must exist.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TransactionListWithProof {
    pub transactions: Vec<Transaction>,
    pub events: Option<Vec<Vec<ContractEvent>>>,
    pub first_transaction_version: Option<Version>,
    pub proof: TransactionListProof,
}

impl TransactionListWithProof {
    /// Constructor.
    pub fn new(
        transactions: Vec<Transaction>,
        events: Option<Vec<Vec<ContractEvent>>>,
        first_transaction_version: Option<Version>,
        proof: TransactionListProof,
    ) -> Self {
        Self {
            transactions,
            events,
            first_transaction_version,
            proof,
        }
    }

    /// Creates an empty transaction list.
    pub fn new_empty() -> Self {
        Self::new(vec![], None, None, TransactionListProof::new_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    /// Verifies the transaction list with the proofs, both carried on `self`.
    ///
    /// Two things are ensured if no error is raised:
    ///   1. All the transactions exist on the ledger represented by `ledger_info`.
    ///   2. And the transactions in the list has consecutive versions starting from
    /// `first_transaction_version`. When `first_transaction_version` is None, ensures the list is
    /// empty.
    pub fn verify(
        &self,
        ledger_info: &LedgerInfo,
        first_transaction_version: Option<Version>,
    ) -> Result<()> {
        ensure!(
            self.first_transaction_version == first_transaction_version,
            "First transaction version ({}) not expected ({}).",
            Self::display_option_version(self.first_transaction_version),
            Self::display_option_version(first_transaction_version),
        );

        let txn_hashes: Vec<_> = self.transactions.iter().map(CryptoHash::hash).collect();
        self.proof
            .verify(ledger_info, self.first_transaction_version, &txn_hashes)?;

        // Verify the events if they exist.
        if let Some(event_lists) = &self.events {
            ensure!(
                event_lists.len() == self.transactions.len(),
                "The length of event_lists ({}) does not match the number of transactions ({}).",
                event_lists.len(),
                self.transactions.len(),
            );
            itertools::zip_eq(event_lists, self.proof.transaction_infos())
                .map(|(events, txn_info)| {
                    let event_hashes: Vec<_> = events.iter().map(ContractEvent::hash).collect();
                    let event_root_hash =
                        InMemoryAccumulator::<EventAccumulatorHasher>::from_leaves(&event_hashes)
                            .root_hash();
                    ensure!(
                        event_root_hash == txn_info.event_root_hash(),
                        "Some event root hash calculated doesn't match that carried on the \
                         transaction info.",
                    );
                    Ok(())
                })
                .collect::<Result<Vec<_>>>()?;
        }

        Ok(())
    }

    fn display_option_version(version: Option<Version>) -> String {
        match version {
            Some(v) => format!("{}", v),
            None => String::from("absent"),
        }
    }
}

/// A list of transactions under an account that are contiguous by sequence number
/// and include proofs.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
pub struct AccountTransactionsWithProof(pub Vec<TransactionWithProof>);

impl AccountTransactionsWithProof {
    pub fn new(txns_with_proofs: Vec<TransactionWithProof>) -> Self {
        Self(txns_with_proofs)
    }

    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn inner(&self) -> &[TransactionWithProof] {
        &self.0
    }

    pub fn into_inner(self) -> Vec<TransactionWithProof> {
        self.0
    }

    // TODO(philiphayes): this will need to change to support CRSNs
    // (Conflict-Resistant Sequence Numbers)[https://github.com/diem/dip/blob/main/dips/dip-168.md].
    //
    // If we use a separate event stream under each account for sequence numbers,
    // we'll probably need to always `include_events: true`, find the sequence
    // number event, and use that to guarantee the response is sequential and
    // complete.
    /// 1. Verify all transactions are consistent with the given ledger info.
    /// 2. All transactions were sent by `account`.
    /// 3. The transactions are contiguous by sequence number, starting at `start_seq_num`.
    /// 4. No more transactions than limit.
    /// 5. Events are present when requested (and not present when not requested).
    /// 6. Transactions are not newer than requested ledger version.
    pub fn verify(
        &self,
        ledger_info: &LedgerInfo,
        account: AccountAddress,
        start_seq_num: u64,
        limit: u64,
        include_events: bool,
        ledger_version: Version,
    ) -> Result<()> {
        ensure!(
            self.len() as u64 <= limit,
            "number of account transactions ({}) exceeded limit ({})",
            self.len(),
            limit,
        );

        self.0
            .iter()
            .enumerate()
            .try_for_each(|(seq_num_offset, txn_with_proof)| {
                let expected_seq_num = start_seq_num.saturating_add(seq_num_offset as u64);
                let txn_version = txn_with_proof.version;

                ensure!(
                    include_events == txn_with_proof.events.is_some(),
                    "unexpected events or missing events"
                );
                ensure!(
                    txn_version <= ledger_version,
                    "transaction with version ({}) greater than requested ledger version ({})",
                    txn_version,
                    ledger_version,
                );

                txn_with_proof.verify_user_txn(ledger_info, txn_version, account, expected_seq_num)
            })
    }
}

/// `Transaction` will be the transaction type used internally in the diem node to represent the
/// transaction to be processed and persisted.
///
/// We suppress the clippy warning here as we would expect most of the transaction to be user
/// transaction.
#[allow(clippy::large_enum_variant)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, CryptoHasher, BCSCryptoHash)]
pub enum Transaction {
    /// Transaction submitted by the user. e.g: P2P payment transaction, publishing module
    /// transaction, etc.
    /// TODO: We need to rename SignedTransaction to SignedUserTransaction, as well as all the other
    ///       transaction types we had in our codebase.
    UserTransaction(SignedTransaction),

    /// Transaction that applies a WriteSet to the current storage, it's applied manually via db-bootstrapper.
    GenesisTransaction(WriteSetPayload),

    /// Transaction to update the block metadata resource at the beginning of a block.
    BlockMetadata(BlockMetadata),
}

impl Transaction {
    pub fn as_signed_user_txn(&self) -> Result<&SignedTransaction> {
        match self {
            Transaction::UserTransaction(txn) => Ok(txn),
            _ => Err(format_err!("Not a user transaction.")),
        }
    }

    pub fn format_for_client(&self, get_transaction_name: impl Fn(&[u8]) -> String) -> String {
        match self {
            Transaction::UserTransaction(user_txn) => {
                user_txn.format_for_client(get_transaction_name)
            }
            // TODO: display proper information for client
            Transaction::GenesisTransaction(_write_set) => String::from("genesis"),
            // TODO: display proper information for client
            Transaction::BlockMetadata(_block_metadata) => String::from("block_metadata"),
        }
    }
}

impl TryFrom<Transaction> for SignedTransaction {
    type Error = Error;

    fn try_from(txn: Transaction) -> Result<Self> {
        match txn {
            Transaction::UserTransaction(txn) => Ok(txn),
            _ => Err(format_err!("Not a user transaction.")),
        }
    }
}
