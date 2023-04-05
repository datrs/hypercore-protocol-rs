use curve25519_dalek::scalar::Scalar;
use hypercore::{generate_keypair, PublicKey, SecretKey};
use sha2::Digest;
use snow::{
    params::{CipherChoice, DHChoice, HashChoice},
    resolvers::CryptoResolver,
    types::{Cipher, Dh, Hash, Random},
};
use std::convert::TryInto;

/// Wraps ed25519-dalek compatible keypair
#[derive(Default)]
struct Ed25519 {
    privkey: [u8; 32],
    pubkey: [u8; 32],
}

impl Dh for Ed25519 {
    fn name(&self) -> &'static str {
        "Ed25519"
    }

    fn pub_len(&self) -> usize {
        32
    }

    fn priv_len(&self) -> usize {
        32
    }

    fn set(&mut self, privkey: &[u8]) {
        let secret: SecretKey =
            SecretKey::from_bytes(privkey).expect("Can't use given bytes as SecretKey");
        let public: PublicKey = (&secret).into();
        self.privkey[..privkey.len()].copy_from_slice(privkey);
        let public_key_bytes = public.as_bytes();
        self.pubkey[..public_key_bytes.len()].copy_from_slice(public_key_bytes);
    }

    fn generate(&mut self, _: &mut dyn Random) {
        // NB: Given Random can't be used with eed25519_dalek's Keypair::generate(),
        // use OS's random here.
        let key_pair = generate_keypair();
        let secret_key_bytes = key_pair.secret.as_bytes();
        self.privkey[..secret_key_bytes.len()].copy_from_slice(secret_key_bytes);
        let public_key_bytes = key_pair.public.as_bytes();
        self.pubkey[..public_key_bytes.len()].copy_from_slice(public_key_bytes);
    }

    fn pubkey(&self) -> &[u8] {
        &self.pubkey
    }

    fn privkey(&self) -> &[u8] {
        &self.privkey
    }

    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let sk: [u8; 32] = sha2::Sha512::digest(self.privkey).as_slice()[..32]
            .try_into()
            .unwrap();
        let sk = clamp_scalar(sk);
        // PublicKey is a CompressedEdwardsY in dalek. So we decompress it to get the
        // EdwardsPoint and use variable base multiplication.
        let cey =
            curve25519_dalek::edwards::CompressedEdwardsY::from_slice(&pubkey[..self.pub_len()])
                .map_err(|_| snow::Error::Dh)?;
        let pubkey: curve25519_dalek::edwards::EdwardsPoint = match cey.decompress() {
            Some(ep) => Ok(ep),
            None => Err(snow::Error::Dh),
        }?;
        let result = pubkey * sk;
        let result: [u8; 32] = *result.compress().as_bytes();
        out[..result.len()].copy_from_slice(result.as_slice());
        Ok(())
    }
}

#[derive(Default)]
pub struct CurveResolver;

impl CryptoResolver for CurveResolver {
    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        match *choice {
            DHChoice::Curve25519 => Some(Box::<Ed25519>::default()),
            _ => None,
        }
    }

    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        None
    }

    fn resolve_hash(&self, _choice: &HashChoice) -> Option<Box<dyn Hash>> {
        None
    }

    fn resolve_cipher(&self, _choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        None
    }
}

fn clamp_scalar(mut scalar: [u8; 32]) -> Scalar {
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;

    Scalar::from_bits(scalar)
}
