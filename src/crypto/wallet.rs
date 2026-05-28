use bech32::{primitives::decode::CheckedHrpstring, Bech32m, Hrp};
use bip32::{DerivationPath, XPrv};
use bip39::{Language, Mnemonic};
use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use sha3::{Digest, Sha3_256};

use crate::consensus::UNITS_PER_DBC;

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("invalid mnemonic: {0}")]
    Mnemonic(String),
    #[error("bip32 derivation error: {0}")]
    Bip32(String),
    #[error("bech32 error: {0}")]
    Bech32(String),
    #[error("invalid address HRP (expected dbc)")]
    BadHrp,
    #[error("invalid address length")]
    BadLength,
    #[error("invalid signature")]
    InvalidSignature,
}

/// A DBC address payload: 20-byte hash (SHA3-256 then BLAKE3, truncated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 20]);

impl Address {
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    pub fn to_bech32m(&self) -> Result<String, WalletError> {
        let hrp = Hrp::parse("dbc").map_err(|e| WalletError::Bech32(e.to_string()))?;
        bech32::encode::<Bech32m>(hrp, &self.0).map_err(|e| WalletError::Bech32(e.to_string()))
    }

    pub fn from_bech32m(s: &str) -> Result<Self, WalletError> {
        let checked = CheckedHrpstring::new::<Bech32m>(s)
            .map_err(|e| WalletError::Bech32(e.to_string()))?;
        if checked.hrp().as_str() != "dbc" {
            return Err(WalletError::BadHrp);
        }
        let bytes = checked.byte_iter().collect::<Vec<u8>>();
        if bytes.len() != 20 {
            return Err(WalletError::BadLength);
        }
        let mut out = [0u8; 20];
        out.copy_from_slice(&bytes);
        Ok(Address(out))
    }
}

/// A wallet keypair + derived address.
pub struct Wallet {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
    address: Address,
}

impl Wallet {
    /// Generate a fresh 24-word BIP-39 mnemonic.
    pub fn generate_mnemonic() -> Mnemonic {
        // Uses OS RNG via the `rand` feature.
        Mnemonic::generate_in(Language::English, 24).expect("mnemonic generation must work")
    }

    /// Create a wallet from a mnemonic and a fixed derivation path.
    ///
    /// Whitepaper calls for BIP-32/BIP-39. We use a simple default path.
    pub fn from_mnemonic(mnemonic: &Mnemonic) -> Result<Self, WalletError> {
        let seed = mnemonic.to_seed("");
        // Default-ish path. Coin type is TBD in whitepaper; using 0 for now.
        let path: DerivationPath = "m/44'/0'/0'/0/0"
            .parse::<DerivationPath>()
            .map_err(|e| WalletError::Bip32(e.to_string()))?;
        let xprv = XPrv::derive_from_path(seed.as_slice(), &path)
            .map_err(|e| WalletError::Bip32(e.to_string()))?;

        let sk = SigningKey::from_bytes(xprv.private_key().to_bytes().as_slice().into())
            .map_err(|e| WalletError::Bip32(e.to_string()))?;
        let vk = sk.verifying_key().clone();
        let addr = address_from_verifying_key(&vk);
        Ok(Wallet {
            signing_key: sk,
            verifying_key: vk,
            address: addr,
        })
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    pub fn sign(&self, msg32: &[u8; 32]) -> Signature {
        use k256::schnorr::signature::Signer;
        self.signing_key.sign(msg32)
    }

    pub fn verify(&self, msg32: &[u8; 32], sig: &Signature) -> Result<(), WalletError> {
        use k256::schnorr::signature::Verifier;
        self.verifying_key
            .verify(msg32, sig)
            .map_err(|_| WalletError::InvalidSignature)
    }

    pub fn pubkey32(&self) -> [u8; 32] {
        self.verifying_key.to_bytes().into()
    }
}

pub fn address_from_verifying_key(vk: &VerifyingKey) -> Address {
    // Schnorr verifying key serializes as 32-byte x-only public key.
    let pubkey = vk.to_bytes();

    // Whitepaper: "SHA-3 + BLAKE3". We do Sha3_256(pubkey) then Blake3(hash) and truncate to 20 bytes.
    let sha = Sha3_256::digest(pubkey);
    let b3 = blake3::hash(&sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&b3.as_bytes()[..20]);
    Address(out)
}

/// Convenience for UI/CLI: format a balance in DBC with 8 decimals.
pub fn format_units_as_dbc(units: u64) -> String {
    let whole = units / UNITS_PER_DBC;
    let frac = units % UNITS_PER_DBC;
    format!("{whole}.{frac:08}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_roundtrip_bech32m() {
        let m = Wallet::generate_mnemonic();
        let w = Wallet::from_mnemonic(&m).unwrap();
        let s = w.address().to_bech32m().unwrap();
        assert!(s.starts_with("dbc1"));
        let a2 = Address::from_bech32m(&s).unwrap();
        assert_eq!(a2, w.address());
    }

    #[test]
    fn sign_verify_works() {
        let m = Wallet::generate_mnemonic();
        let w = Wallet::from_mnemonic(&m).unwrap();
        let msg = blake3::hash(b"hello").as_bytes().to_owned();
        let msg32: [u8; 32] = msg;
        let sig = w.sign(&msg32);
        w.verify(&msg32, &sig).unwrap();
    }
}

