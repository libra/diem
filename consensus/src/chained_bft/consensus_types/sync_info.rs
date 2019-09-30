use crate::chained_bft::consensus_types::{
    quorum_cert::QuorumCert, timeout_msg::PacemakerTimeoutCertificate,
};
use libra_network;

use crate::chained_bft::common::Round;
use failure::ResultExt;
use libra_proto_conv::{FromProto, IntoProto};
use libra_types::crypto_proxies::ValidatorVerifier;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
/// This struct describes basic synchronization metadata.
pub struct SyncInfo {
    /// Highest quorum certificate known to the peer.
    highest_quorum_cert: QuorumCert,
    /// Highest ledger info known to the peer.
    highest_ledger_info: QuorumCert,
    /// Optional highest timeout certificate if available.
    highest_timeout_cert: Option<PacemakerTimeoutCertificate>,
}

impl Display for SyncInfo {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let htc_repr = match self.highest_timeout_certificate() {
            Some(tc) => format!("TC for round {}", tc.round()),
            None => "None".to_string(),
        };
        write!(
            f,
            "SyncInfo[round: {}, HQC: {}, HLI: {}, HTC: {}]",
            self.highest_round(),
            self.highest_quorum_cert,
            self.highest_ledger_info,
            htc_repr,
        )
    }
}

impl SyncInfo {
    pub fn new(
        highest_quorum_cert: QuorumCert,
        highest_ledger_info: QuorumCert,
        highest_timeout_cert: Option<PacemakerTimeoutCertificate>,
    ) -> Self {
        Self {
            highest_quorum_cert,
            highest_ledger_info,
            highest_timeout_cert,
        }
    }

    /// Highest quorum certificate
    pub fn highest_quorum_cert(&self) -> &QuorumCert {
        &self.highest_quorum_cert
    }

    /// Highest ledger info
    pub fn highest_ledger_info(&self) -> &QuorumCert {
        &self.highest_ledger_info
    }

    /// Highest timeout certificate if available
    pub fn highest_timeout_certificate(&self) -> Option<&PacemakerTimeoutCertificate> {
        self.highest_timeout_cert.as_ref()
    }

    pub fn hqc_round(&self) -> Round {
        self.highest_quorum_cert.certified_block_round()
    }

    pub fn htc_round(&self) -> Round {
        self.highest_timeout_certificate()
            .map_or(0, |tc| tc.round())
    }

    /// The highest round the SyncInfo carries.
    pub fn highest_round(&self) -> Round {
        std::cmp::max(self.hqc_round(), self.htc_round())
    }

    pub fn verify(&self, validator: &ValidatorVerifier) -> failure::Result<()> {
        self.highest_quorum_cert
            .verify(validator)
            .and_then(|_| self.highest_ledger_info.verify(validator))
            .and_then(|_| {
                if let Some(tc) = &self.highest_timeout_cert {
                    tc.verify(validator)?;
                }
                Ok(())
            })
            .with_context(|e| format!("Fail to verify SyncInfo: {:?}", e))?;
        Ok(())
    }
}

impl FromProto for SyncInfo {
    type ProtoType = libra_network::proto::SyncInfo;

    fn from_proto(mut object: libra_network::proto::SyncInfo) -> failure::Result<Self> {
        let highest_quorum_cert = QuorumCert::from_proto(object.take_highest_quorum_cert())?;
        let highest_ledger_info = QuorumCert::from_proto(object.take_highest_ledger_info())?;
        let highest_timeout_cert = if let Some(tc) = object.highest_timeout_cert.into_option() {
            Some(PacemakerTimeoutCertificate::from_proto(tc)?)
        } else {
            None
        };
        Ok(SyncInfo::new(
            highest_quorum_cert,
            highest_ledger_info,
            highest_timeout_cert,
        ))
    }
}
impl IntoProto for SyncInfo {
    type ProtoType = libra_network::proto::SyncInfo;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        proto.set_highest_quorum_cert(self.highest_quorum_cert.into_proto());
        proto.set_highest_ledger_info(self.highest_ledger_info.into_proto());
        if let Some(tc) = self.highest_timeout_cert {
            proto.set_highest_timeout_cert(tc.into_proto());
        }
        proto
    }
}
