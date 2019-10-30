// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    account_address::AccountAddress,
    transaction::Version,
    validator_set::ValidatorSet,
    validator_verifier::{ValidatorVerifier, VerifyError},
};
use canonical_serialization::{CanonicalSerialize, CanonicalSerializer, SimpleSerializer};
use crypto::{
    hash::{CryptoHash, CryptoHasher, LedgerInfoHasher},
    HashValue, *,
};
use failure::prelude::*;
#[cfg(any(test, feature = "testing"))]
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    convert::{TryFrom, TryInto},
    fmt::{Display, Formatter},
};

/// This structure serves a dual purpose.
///
/// First, if this structure is signed by 2f+1 validators it signifies the state of the ledger at
/// version `version` -- it contains the transaction accumulator at that version which commits to
/// all historical transactions. This structure may be expanded to include other information that
/// is derived from that accumulator (e.g. the current time according to the time contract) to
/// reduce the number of proofs a client must get.
///
/// Second, the structure contains a `consensus_data_hash` value. This is the hash of an internal
/// data structure that represents a block that is voted on in HotStuff. If 2f+1 signatures are
/// gathered on the same ledger info that represents a Quorum Certificate (QC) on the consensus
/// data.
///
/// Combining these two concepts, when a validator votes on a block, B it votes for a
/// LedgerInfo with the `version` being the latest version that will be committed if B gets 2f+1
/// votes. It sets `consensus_data_hash` to represent B so that if those 2f+1 votes are gathered a
/// QC is formed on B.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "testing"), derive(Arbitrary))]
pub struct LedgerInfo {
    /// The version of the latest transaction in the ledger.
    version: Version,

    /// The root hash of transaction accumulator that includes the latest transaction.
    transaction_accumulator_hash: HashValue,

    /// Hash of consensus specific data that is opaque to all parts of the system other than
    /// consensus.
    consensus_data_hash: HashValue,

    /// Block id of the last committed block corresponding to this LedgerInfo
    /// as reported by consensus.
    consensus_block_id: HashValue,

    /// Epoch number corresponds to the set of validators that are active for this ledger info.
    epoch_num: u64,

    /// Timestamp that represents the microseconds since the epoch (unix time) that is
    /// generated by the proposer of the block.  This is strictly increasing with every block.
    /// If a client reads a timestamp > the one they specified for transaction expiration time,
    /// they can be certain that their transaction will never be included in a block in the future
    /// (assuming that their transaction has not yet been included)
    timestamp_usecs: u64,

    /// An optional field keeping the set of new validators to start the next epoch.
    /// The very last ledger info of an epoch contains the validator set for the next one,
    /// other ledger info instances are None.
    next_validator_set: Option<ValidatorSet>,
}

impl Display for LedgerInfo {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(
            f,
            "LedgerInfo: [committed_block_id: {}, version: {}, epoch_num: {}, timestamp (us): {}, next_validator_set: {}]",
            self.consensus_block_id(),
            self.version(),
            self.epoch_num(),
            self.timestamp_usecs(),
            self.next_validator_set.as_ref().map_or("None".to_string(), |validator_set| format!("{}", validator_set)),
        )
    }
}

impl LedgerInfo {
    /// Constructs a `LedgerInfo` object at a specific version using a given
    /// transaction accumulator root and consensus data hash.
    pub fn new(
        version: Version,
        transaction_accumulator_hash: HashValue,
        consensus_data_hash: HashValue,
        consensus_block_id: HashValue,
        epoch_num: u64,
        timestamp_usecs: u64,
        next_validator_set: Option<ValidatorSet>,
    ) -> Self {
        LedgerInfo {
            version,
            transaction_accumulator_hash,
            consensus_data_hash,
            consensus_block_id,
            epoch_num,
            timestamp_usecs,
            next_validator_set,
        }
    }

    pub fn new_by_version(version: Version, transaction_accumulator_hash:HashValue, li:LedgerInfo) -> Self {
        let v_s = match li.next_validator_set() {
            Some(n_v_s) => {
                Some(n_v_s.clone())
            },
            None => None
        };
        LedgerInfo {
            version,
            transaction_accumulator_hash:transaction_accumulator_hash,
            consensus_data_hash:li.consensus_data_hash(),
            consensus_block_id:li.consensus_block_id(),
            epoch_num:li.epoch_num(),
            timestamp_usecs:li.timestamp_usecs(),
            next_validator_set:v_s,
        }
    }

    /// Returns the version of this `LedgerInfo`.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Returns the transaction accumulator root of this `LedgerInfo`.
    pub fn transaction_accumulator_hash(&self) -> HashValue {
        self.transaction_accumulator_hash
    }

    /// Returns hash of consensus data in this `LedgerInfo`.
    pub fn consensus_data_hash(&self) -> HashValue {
        self.consensus_data_hash
    }

    pub fn consensus_block_id(&self) -> HashValue {
        self.consensus_block_id
    }

    pub fn set_consensus_data_hash(&mut self, consensus_data_hash: HashValue) {
        self.consensus_data_hash = consensus_data_hash;
    }

    pub fn epoch_num(&self) -> u64 {
        self.epoch_num
    }

    pub fn timestamp_usecs(&self) -> u64 {
        self.timestamp_usecs
    }

    /// A ledger info is nominal if it's not certifying any real version.
    pub fn is_zero(&self) -> bool {
        self.version == 0
    }

    pub fn next_validator_set(&self) -> Option<&ValidatorSet> {
        self.next_validator_set.as_ref()
    }
}

impl TryFrom<crate::proto::types::LedgerInfo> for LedgerInfo {
    type Error = Error;

    fn try_from(proto: crate::proto::types::LedgerInfo) -> Result<Self> {
        let version = proto.version;
        let transaction_accumulator_hash =
            HashValue::from_slice(&proto.transaction_accumulator_hash)?;
        let consensus_data_hash = HashValue::from_slice(&proto.consensus_data_hash)?;
        let consensus_block_id = HashValue::from_slice(&proto.consensus_block_id)?;
        let epoch_num = proto.epoch_num;
        let timestamp_usecs = proto.timestamp_usecs;

        let next_validator_set = if let Some(validator_set_proto) = proto.next_validator_set {
            Some(ValidatorSet::try_from(validator_set_proto)?)
        } else {
            None
        };
        Ok(LedgerInfo::new(
            version,
            transaction_accumulator_hash,
            consensus_data_hash,
            consensus_block_id,
            epoch_num,
            timestamp_usecs,
            next_validator_set,
        ))
    }
}

impl From<LedgerInfo> for crate::proto::types::LedgerInfo {
    fn from(ledger_info: LedgerInfo) -> Self {
        Self {
            version: ledger_info.version,
            transaction_accumulator_hash: ledger_info.transaction_accumulator_hash.to_vec(),
            consensus_data_hash: ledger_info.consensus_data_hash.to_vec(),
            consensus_block_id: ledger_info.consensus_block_id.to_vec(),
            epoch_num: ledger_info.epoch_num,
            timestamp_usecs: ledger_info.timestamp_usecs,
            next_validator_set: ledger_info.next_validator_set.map(Into::into),
        }
    }
}

impl CanonicalSerialize for LedgerInfo {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer
            .encode_u64(self.version)?
            .encode_bytes(self.transaction_accumulator_hash.as_ref())?
            .encode_bytes(self.consensus_data_hash.as_ref())?
            .encode_bytes(self.consensus_block_id.as_ref())?
            .encode_u64(self.epoch_num)?
            .encode_u64(self.timestamp_usecs)?
            .encode_optional(&self.next_validator_set)?;
        Ok(())
    }
}

impl CryptoHash for LedgerInfo {
    type Hasher = LedgerInfoHasher;

    fn hash(&self) -> HashValue {
        let mut state = Self::Hasher::default();
        state.write(
            &SimpleSerializer::<Vec<u8>>::serialize(self).expect("Serialization should work."),
        );
        state.finish()
    }
}

/// The validator node returns this structure which includes signatures
/// from validators that confirm the state.  The client needs to only pass back
/// the LedgerInfo element since the validator node doesn't need to know the signatures
/// again when the client performs a query, those are only there for the client
/// to be able to verify the state
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerInfoWithSignatures<Sig> {
    ledger_info: LedgerInfo,
    /// The validator is identified by its account address: in order to verify a signature
    /// one needs to retrieve the public key of the validator for the given epoch.
    signatures: BTreeMap<AccountAddress, Sig>,
}

impl<Sig: CanonicalSerialize> CanonicalSerialize for LedgerInfoWithSignatures<Sig> {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer
            .encode_struct(&self.ledger_info)?
            .encode_btreemap(&self.signatures)?;
        Ok(())
    }
}

impl<Sig> Display for LedgerInfoWithSignatures<Sig> {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(f, "{}", self.ledger_info)
    }
}

impl<Sig: Signature> LedgerInfoWithSignatures<Sig> {
    pub fn new(ledger_info: LedgerInfo, signatures: BTreeMap<AccountAddress, Sig>) -> Self {
        LedgerInfoWithSignatures {
            ledger_info,
            signatures,
        }
    }

    pub fn ledger_info(&self) -> &LedgerInfo {
        &self.ledger_info
    }

    pub fn add_signature(&mut self, validator: AccountAddress, signature: Sig) {
        self.signatures.entry(validator).or_insert(signature);
    }

    pub fn remove_signature(&mut self, validator: AccountAddress) {
        self.signatures.remove(&validator);
    }

    pub fn signatures(&self) -> &BTreeMap<AccountAddress, Sig> {
        &self.signatures
    }

    pub fn verify(
        &self,
        validator: &ValidatorVerifier<Sig::VerifyingKeyMaterial>,
    ) -> ::std::result::Result<(), VerifyError> {
        if self.ledger_info.is_zero() {
            // We're not trying to verify nominal ledger info that does not carry any information.
            return Ok(());
        }
        let ledger_hash = self.ledger_info().hash();
        validator.batch_verify_aggregated_signature(ledger_hash, self.signatures())
    }
}

impl<Sig: Signature> TryFrom<crate::proto::types::LedgerInfoWithSignatures>
    for LedgerInfoWithSignatures<Sig>
{
    type Error = Error;

    fn try_from(proto: crate::proto::types::LedgerInfoWithSignatures) -> Result<Self> {
        let ledger_info = proto
            .ledger_info
            .ok_or_else(|| format_err!("Missing ledger_info"))?
            .try_into()?;

        let signatures_proto = proto.signatures;
        let num_signatures = signatures_proto.len();
        let signatures = signatures_proto
            .into_iter()
            .map(|proto| {
                let validator_id = AccountAddress::try_from(proto.validator_id)?;
                let signature_bytes: &[u8] = proto.signature.as_ref();
                let signature = Sig::try_from(signature_bytes)?;
                Ok((validator_id, signature))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        ensure!(
            signatures.len() == num_signatures,
            "Signatures should be from different validators."
        );

        Ok(LedgerInfoWithSignatures {
            ledger_info,
            signatures,
        })
    }
}

impl<Sig: Signature> From<LedgerInfoWithSignatures<Sig>>
    for crate::proto::types::LedgerInfoWithSignatures
{
    fn from(ledger_info_with_sigs: LedgerInfoWithSignatures<Sig>) -> Self {
        let ledger_info = Some(ledger_info_with_sigs.ledger_info.into());
        let signatures = ledger_info_with_sigs
            .signatures
            .into_iter()
            .map(
                |(validator_id, signature)| crate::proto::types::ValidatorSignature {
                    validator_id: validator_id.to_vec(),
                    signature: signature.to_bytes().to_vec(),
                },
            )
            .collect();

        Self {
            signatures,
            ledger_info,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ledger_info::{LedgerInfo, LedgerInfoWithSignatures};
    use crate::validator_signer::ValidatorSigner;
    use canonical_serialization::SimpleSerializer;
    use crypto::{ed25519::*, HashValue};
    use std::collections::BTreeMap;

    #[test]
    fn test_signatures_hash() {
        let ledger_info = LedgerInfo::new(
            0,
            HashValue::zero(),
            HashValue::zero(),
            HashValue::zero(),
            0,
            0,
            None,
        );

        let random_hash = HashValue::random();
        const NUM_SIGNERS: u8 = 7;
        // Generate NUM_SIGNERS random signers.
        let validator_signers: Vec<ValidatorSigner<Ed25519PrivateKey>> = (0..NUM_SIGNERS)
            .map(|i| ValidatorSigner::random([i; 32]))
            .collect();
        let mut author_to_signature_map = BTreeMap::new();
        for validator in validator_signers.iter() {
            author_to_signature_map.insert(
                validator.author(),
                validator.sign_message(random_hash).unwrap(),
            );
        }

        let ledger_info_with_signatures =
            LedgerInfoWithSignatures::new(ledger_info.clone(), author_to_signature_map);

        // Add the signatures in reverse order and ensure the serialization matches
        let mut author_to_signature_map = BTreeMap::new();
        for validator in validator_signers.iter().rev() {
            author_to_signature_map.insert(
                validator.author(),
                validator.sign_message(random_hash).unwrap(),
            );
        }

        let ledger_info_with_signatures_reversed =
            LedgerInfoWithSignatures::new(ledger_info.clone(), author_to_signature_map);

        let ledger_info_with_signatures_bytes =
            SimpleSerializer::<Vec<u8>>::serialize(&ledger_info_with_signatures)
                .expect("block serialization failed");
        let ledger_info_with_signatures_reversed_bytes =
            SimpleSerializer::<Vec<u8>>::serialize(&ledger_info_with_signatures_reversed)
                .expect("block serialization failed");

        assert_eq!(
            ledger_info_with_signatures_bytes,
            ledger_info_with_signatures_reversed_bytes
        );
    }
}
