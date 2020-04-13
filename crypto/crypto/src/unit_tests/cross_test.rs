// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

// This is necessary for the derive macros which rely on being used in a
// context where the crypto crate is external
use crate as libra_crypto;
use crate::{ed25519, multi_ed25519, test_utils::uniform_keypair_strategy, traits::*};

use crate::hash::HashValue;

use libra_crypto_derive::{
    PrivateKeyExt, PublicKeyExt, SignatureExt, SigningKey, SilentDebug, ValidKey, VerifyingKey,
};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};

// Here we aim to make a point about how we can build an enum generically
// on top of a few choice signing scheme types. This enum implements the
// VerifyingKey, SigningKey for precisely the types selected for that enum
// (here, Ed25519(PublicKey|PrivateKey|Signature) and MultiEd25519(...)).
//
// Note that we do not break type safety (see towards the end), and that
// this enum can safely be put into the usual collection (Vec, HashMap).
//

#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, ValidKey, PublicKeyExt, VerifyingKey,
)]
#[PrivateKeyType = "PrivateK"]
#[SignatureType = "Sig"]
enum PublicK {
    Ed(ed25519::PublicKey),
    MultiEd(PublicKey),
}

#[derive(Serialize, Deserialize, SilentDebug, ValidKey, PrivateKeyExt, SigningKey)]
#[PublicKeyType = "PublicK"]
#[SignatureType = "Sig"]
enum PrivateK {
    Ed(ed25519::PrivateKey),
    MultiEd(PrivateKey),
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq, Hash, SignatureExt)]
#[PublicKeyType = "PublicK"]
#[PrivateKeyType = "PrivateK"]
enum Sig {
    Ed(ed25519::Signature),
    MultiEd(Signature),
}

///////////////////////////////////////////////////////
// End of declarations — let's now prove type safety //
///////////////////////////////////////////////////////
proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    #[test]
    fn test_keys_mix(
        hash in any::<HashValue>(),
        ed_keypair1 in uniform_keypair_strategy::<ed25519::PrivateKey, ed25519::PublicKey>(),
        ed_keypair2 in uniform_keypair_strategy::<ed25519::PrivateKey, ed25519::PublicKey>(),
        med_keypair in uniform_keypair_strategy::<multi_ed25519::PrivateKey, multi_ed25519::PublicKey>()
    ) {
        // this is impossible to write statically, due to the trait not being
        // object-safe (voluntarily)
        // let mut l: Vec<Box<dyn PrivateKey>> = vec![];
        let mut l: Vec<ed25519::PrivateKey> = vec![];
        l.push(ed_keypair1.private_key);
        let ed_key = l.pop().unwrap();
        let signature = ed_key.sign_message(&hash);

        // This is business as usual
        prop_assert!(signature.verify(&hash, &ed_keypair1.public_key).is_ok());

        // This is impossible to write, and generates:
        // expected struct `ed25519::ed25519::PublicKey`, found struct `med12381::multi_ed25519::PublicKey`
        // prop_assert!(signature.verify(&hash, &med_keypair.public_key).is_ok());

        let mut l2: Vec<PrivateK> = vec![];
        l2.push(PrivateK::MultiEd(med_keypair.private_key));
        l2.push(PrivateK::Ed(ed_keypair2.private_key));

        let ed_key = l2.pop().unwrap();
        let ed_signature = ed_key.sign_message(&hash);

        // This is still business as usual
        let ed_pubkey2 = PublicK::Ed(ed_keypair2.public_key);
        let good_sigver = ed_signature.verify(&hash, &ed_pubkey2);
        prop_assert!(good_sigver.is_ok(), "{:?}", good_sigver);

        // but this still fails, as expected
        let med_pubkey = PublicK::MultiEd(med_keypair.public_key);
        let bad_sigver = ed_signature.verify(&hash, &med_pubkey);
        prop_assert!(bad_sigver.is_err(), "{:?}", bad_sigver);

        // And now just in case we're confused again, we pop in the
        // reverse direction
        let med_key = l2.pop().unwrap();
        let med_signature = med_key.sign_message(&hash);

        // This is still business as usual
        let good_sigver = med_signature.verify(&hash, &med_pubkey);
        prop_assert!(good_sigver.is_ok(), "{:?}", good_sigver);

        // but this still fails, as expected
        let bad_sigver = med_signature.verify(&hash, &ed_pubkey2);
        prop_assert!(bad_sigver.is_err(), "{:?}", bad_sigver);
    }
}
