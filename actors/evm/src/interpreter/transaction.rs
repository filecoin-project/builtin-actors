#![allow(dead_code)]

use {
    crate::interpreter::{H160, H256, U256},
    bytes::Bytes,
    fil_actors_runtime::ActorError,
    fvm_shared::crypto::signature::SECP_PUB_LEN,
    rlp::{DecoderError, Rlp, RlpStream},
    sha3::{Digest, Keccak256},
    std::{fmt::Debug, ops::Deref},
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TransactionAction {
    Call(H160),
    Create,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AccessListItem {
    pub address: H160,
    pub slots: Vec<H256>,
}

pub enum Transaction {
    Legacy {
        chain_id: Option<u64>,
        nonce: u64,
        gas_price: U256,
        gas_limit: u64,
        action: TransactionAction,
        value: U256,
        input: Bytes,
    },
    EIP2930 {
        chain_id: u64,
        nonce: u64,
        gas_price: U256,
        gas_limit: u64,
        action: TransactionAction,
        value: U256,
        input: Bytes,
        access_list: Vec<AccessListItem>,
    },
    EIP1559 {
        chain_id: u64,
        nonce: u64,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
        gas_limit: u64,
        action: TransactionAction,
        value: U256,
        input: Bytes,
        access_list: Vec<AccessListItem>,
    },
}

#[derive(Debug)]
pub struct TransactionRecoveryId(pub u64);

#[derive(Debug)]
pub struct TransactionSignature {
    pub v: TransactionRecoveryId,
    pub r: H256,
    pub s: H256,
}

#[derive(Debug)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: TransactionSignature,
}

impl rlp::Encodable for TransactionAction {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            Self::Call(address) => {
                s.encoder().encode_value(&address[..]);
            }
            Self::Create => s.encoder().encode_value(&[]),
        }
    }
}

impl rlp::Decodable for TransactionAction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.is_empty() {
            if rlp.is_data() {
                Ok(TransactionAction::Create)
            } else {
                Err(DecoderError::RlpExpectedToBeData)
            }
        } else {
            Ok(TransactionAction::Call(rlp.as_val()?))
        }
    }
}

impl rlp::Encodable for AccessListItem {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(2);
        s.append(&self.address);
        s.append_list(&self.slots);
    }
}

impl rlp::Decodable for AccessListItem {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        Ok(Self { address: rlp.val_at(0)?, slots: rlp.list_at(1)? })
    }
}

impl Transaction {
    /// Calculates the hash of the transaction fields without the signature.
    /// This value is the input to the signing function and the signature
    /// is caluculated over this hash and the sender private key.
    pub fn hash(&self) -> H256 {
        let mut s = RlpStream::new();
        match self {
            Transaction::Legacy { chain_id, nonce, gas_price, gas_limit, action, value, input } => {
                if let Some(chain_id) = chain_id {
                    s.begin_list(9);
                    s.append(nonce);
                    s.append(gas_price);
                    s.append(gas_limit);
                    s.append(action);
                    s.append(value);
                    s.append(input);
                    s.append(chain_id);
                    s.append(&0_u8);
                    s.append(&0_u8);
                } else {
                    s.begin_list(6);
                    s.append(nonce);
                    s.append(gas_limit);
                    s.append(gas_limit);
                    s.append(action);
                    s.append(value);
                    s.append(input);
                }
            }
            Transaction::EIP2930 {
                chain_id,
                nonce,
                gas_price,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => {
                s.append_raw(&[1u8], 0);
                s.begin_list(8);
                s.append(chain_id);
                s.append(nonce);
                s.append(gas_price);
                s.append(gas_limit);
                s.append(action);
                s.append(value);
                s.append(input);
                s.append_list(access_list);
            }
            Transaction::EIP1559 {
                chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => {
                s.append_raw(&[2u8], 0);
                s.begin_list(9);
                s.append(chain_id);
                s.append(nonce);
                s.append(max_priority_fee_per_gas);
                s.append(max_fee_per_gas);
                s.append(gas_limit);
                s.append(action);
                s.append(value);
                s.append(input);
                s.append_list(access_list);
            }
        };

        H256::from_slice(Keccak256::digest(s.as_raw()).as_slice())
    }

    pub fn nonce(&self) -> u64 {
        *match self {
            Transaction::Legacy { nonce, .. } => nonce,
            Transaction::EIP2930 { nonce, .. } => nonce,
            Transaction::EIP1559 { nonce, .. } => nonce,
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            Transaction::Legacy { chain_id, .. } => *chain_id,
            Transaction::EIP2930 { chain_id, .. } => Some(*chain_id),
            Transaction::EIP1559 { chain_id, .. } => Some(*chain_id),
        }
    }

    pub fn gas_price(&self) -> U256 {
        *match self {
            Transaction::Legacy { gas_price, .. } => gas_price,
            Transaction::EIP2930 { gas_price, .. } => gas_price,
            Transaction::EIP1559 { max_fee_per_gas, .. } => max_fee_per_gas,
        }
    }

    pub fn gas_limit(&self) -> u64 {
        *match self {
            Transaction::Legacy { gas_limit, .. } => gas_limit,
            Transaction::EIP2930 { gas_limit, .. } => gas_limit,
            Transaction::EIP1559 { gas_limit, .. } => gas_limit,
        }
    }

    pub fn action(&self) -> TransactionAction {
        match self {
            Transaction::Legacy { action, .. } => action,
            Transaction::EIP2930 { action, .. } => action,
            Transaction::EIP1559 { action, .. } => action,
        }
        .clone()
    }

    pub fn input(&self) -> Bytes {
        match self {
            Transaction::Legacy { input, .. } => input,
            Transaction::EIP2930 { input, .. } => input,
            Transaction::EIP1559 { input, .. } => input,
        }
        .clone()
    }

    pub fn value(&self) -> U256 {
        *match self {
            Transaction::Legacy { value, .. } => value,
            Transaction::EIP2930 { value, .. } => value,
            Transaction::EIP1559 { value, .. } => value,
        }
    }
}

impl Deref for TransactionRecoveryId {
    type Target = u64;

    fn deref(&self) -> &u64 {
        &self.0
    }
}

impl TransactionRecoveryId {
    pub fn odd_y_parity(&self) -> u8 {
        if self.0 == 27 || self.0 == 28 || self.0 > 36 {
            ((self.0 - 1) % 2) as u8
        } else {
            4
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        if self.0 > 36 {
            Some((self.0 - 35) / 2)
        } else {
            None
        }
    }
}

impl TryFrom<&[u8]> for SignedTransaction {
    type Error = DecoderError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(DecoderError::RlpIsTooShort);
        }

        match value[0] {
            0x01 => parse_eip2930_transaction(value),
            0x02 => parse_eip1559_transaction(value),
            _ => parse_legacy_transaction(value),
        }
    }
}

impl Deref for SignedTransaction {
    type Target = Transaction;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl SignedTransaction {
    /// Creates RLP serialized representation of the transaction.
    /// This value is the input to the hash function that is used
    /// to calculate the final transaction hash as it appears on
    /// blockchain explorers. This representation can be sent directly
    /// to ETH nodes and to the FVM-EVM bridge
    pub fn serialize(&self) -> Vec<u8> {
        let mut s = RlpStream::new();
        match &self.transaction {
            Transaction::Legacy { nonce, gas_price, gas_limit, action, value, input, .. } => {
                s.begin_list(9);
                s.append(nonce);
                s.append(gas_price);
                s.append(gas_limit);
                s.append(action);
                s.append(value);
                s.append(input);
                s.append(&self.signature.v.0);
                s.append(&self.signature.r);
                s.append(&self.signature.s);
            }
            Transaction::EIP2930 {
                chain_id,
                nonce,
                gas_price,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => {
                s.append_raw(&[1u8], 0);
                s.begin_list(11);
                s.append(chain_id);
                s.append(nonce);
                s.append(gas_price);
                s.append(gas_limit);
                s.append(action);
                s.append(value);
                s.append(input);
                s.append_list(access_list);
                s.append(&self.signature.v.0);
                s.append(&self.signature.r);
                s.append(&self.signature.s);
            }
            Transaction::EIP1559 {
                chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => {
                s.append_raw(&[2u8], 0);
                s.begin_list(12);
                s.append(chain_id);
                s.append(nonce);
                s.append(max_priority_fee_per_gas);
                s.append(max_fee_per_gas);
                s.append(gas_limit);
                s.append(action);
                s.append(value);
                s.append(input);
                s.append_list(access_list);
                s.append(&self.signature.v.0);
                s.append(&self.signature.r);
                s.append(&self.signature.s);
            }
        };
        s.as_raw().to_vec()
    }

    pub fn hash(&self) -> H256 {
        H256::from_slice(Keccak256::digest(&self.serialize()).as_slice())
    }

    /// The secp256k1 public key of the transaction sender.
    ///
    /// This public key can used to derive the equivalent Filecoin account
    pub fn sender_public_key(&self) -> Result<[u8; SECP_PUB_LEN], ActorError> {
        todo!();
        // let mut sig = [0u8; 65];
        // sig[..32].copy_from_slice(self.signature.r.as_bytes());
        // sig[32..64].copy_from_slice(self.signature.s.as_bytes());

        // if matches!(self.transaction, Transaction::Legacy { .. }) {
        //   sig[64] = self.signature.v.odd_y_parity();
        // } else {
        //   sig[64] = self.signature.v.0 as u8;
        // }

        // #[cfg(not(test))] // use a syscall to fvm
        // return fvm_sdk::crypto::recover_secp_public_key(
        //   &self.transaction.hash().to_fixed_bytes(),
        //   &sig,
        // )
        // .map_err(|e| {
        //   ActorError::illegal_argument(format!("failed to recover public key: {e:?}"))
        // });

        // #[cfg(test)]
        // // invoke the recovery impl directly as there is not FVM running this code
        // return Ok(
        //   fvm_shared::crypto::signature::ops::recover_secp_public_key(
        //     &self.transaction.hash().to_fixed_bytes(),
        //     &sig,
        //   )
        //   .unwrap()
        //   .serialize(),
        // );
    }

    /// Ethereum sender address which is 20-bytes trimmed keccak256(pubkey)
    pub fn sender_address(&self) -> Result<H160, ActorError> {
        let pubkey = self.sender_public_key()?;
        let address_slice = &Keccak256::digest(&pubkey[1..])[12..];
        Ok(H160::from_slice(address_slice))
    }
}

/// rlp([nonce, gasPrice, gasLimit, to, value, data, init, v, r, s])
fn parse_legacy_transaction(bytes: &[u8]) -> Result<SignedTransaction, DecoderError> {
    let rlp = Rlp::new(bytes);

    if rlp.item_count()? != 9 {
        return Err(DecoderError::RlpIncorrectListLen);
    }

    let signature = TransactionSignature {
        v: TransactionRecoveryId(rlp.val_at(6)?),
        r: rlp.val_at(7)?,
        s: rlp.val_at(8)?,
    };

    Ok(SignedTransaction {
        transaction: Transaction::Legacy {
            chain_id: signature.v.chain_id(),
            nonce: rlp.val_at(0)?,
            gas_price: rlp.val_at(1)?,
            gas_limit: rlp.val_at(2)?,
            action: rlp.val_at(3)?,
            value: rlp.val_at(4)?,
            input: rlp.val_at(5)?,
        },
        signature,
    })
}

/// 0x01 || rlp([chainId, nonce, gasPrice, gasLimit, to, value, data,
/// accessList, signatureYParity, signatureR, signatureS])
fn parse_eip2930_transaction(bytes: &[u8]) -> Result<SignedTransaction, DecoderError> {
    let rlp = Rlp::new(&bytes[1..]);

    if rlp.item_count()? != 11 {
        return Err(DecoderError::RlpIncorrectListLen);
    }

    let signature = TransactionSignature {
        v: TransactionRecoveryId(rlp.val_at(8)?),
        r: rlp.val_at(9)?,
        s: rlp.val_at(10)?,
    };

    Ok(SignedTransaction {
        transaction: Transaction::EIP2930 {
            chain_id: rlp.val_at(0)?,
            nonce: rlp.val_at(1)?,
            gas_price: rlp.val_at(2)?,
            gas_limit: rlp.val_at(3)?,
            action: rlp.val_at(4)?,
            value: rlp.val_at(5)?,
            input: rlp.val_at(6)?,
            access_list: rlp.list_at(7)?,
        },
        signature,
    })
}

/// 0x02 || rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
/// gas_limit, destination, amount, data, access_list, signature_y_parity,
/// signature_r, signature_s])
fn parse_eip1559_transaction(bytes: &[u8]) -> Result<SignedTransaction, DecoderError> {
    let rlp = Rlp::new(&bytes[1..]);

    if rlp.item_count()? != 12 {
        return Err(DecoderError::RlpIncorrectListLen);
    }

    Ok(SignedTransaction {
        signature: TransactionSignature {
            v: TransactionRecoveryId(rlp.val_at(9)?),
            r: rlp.val_at(10)?,
            s: rlp.val_at(11)?,
        },
        transaction: Transaction::EIP1559 {
            chain_id: rlp.val_at(0)?,
            nonce: rlp.val_at(1)?,
            max_priority_fee_per_gas: rlp.val_at(2)?,
            max_fee_per_gas: rlp.val_at(3)?,
            gas_limit: rlp.val_at(4)?,
            action: rlp.val_at(5)?,
            value: rlp.val_at(6)?,
            input: rlp.val_at(7)?,
            access_list: rlp.list_at(8)?,
        },
    })
}

impl Debug for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Legacy { chain_id, nonce, gas_price, gas_limit, action, value, input } => f
                .debug_struct("Legacy")
                .field("chain_id", chain_id)
                .field("nonce", nonce)
                .field("gas_price", gas_price)
                .field("gas_limit", gas_limit)
                .field("action", action)
                .field("value", value)
                .field("input", &hex::encode(&input))
                .finish(),
            Self::EIP2930 {
                chain_id,
                nonce,
                gas_price,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => f
                .debug_struct("EIP2930")
                .field("chain_id", chain_id)
                .field("nonce", nonce)
                .field("gas_price", gas_price)
                .field("gas_limit", gas_limit)
                .field("action", action)
                .field("value", value)
                .field("input", &hex::encode(&input))
                .field("access_list", access_list)
                .finish(),
            Self::EIP1559 {
                chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit,
                action,
                value,
                input,
                access_list,
            } => f
                .debug_struct("EIP1559")
                .field("chain_id", chain_id)
                .field("nonce", nonce)
                .field("max_priority_fee_per_gas", max_priority_fee_per_gas)
                .field("max_fee_per_gas", max_fee_per_gas)
                .field("gas_limit", gas_limit)
                .field("action", action)
                .field("value", value)
                .field("input", &hex::encode(&input))
                .field("access_list", access_list)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        crate::interpreter::{
            transaction::{AccessListItem, Transaction, TransactionAction},
            SignedTransaction, H160, H256, U256,
        },
        hex_literal::hex,
    };

    #[test]
    fn decode_legacy_transaction() {
        // https://etherscan.io/tx/0x3741aea434dc6e9e740be0113af4bac372fcdd2fa2188409c93c9405cbdcaaf0
        let raw = hex!(
            "f9016b0885113abe69b38302895c947a250d5630b4cf539739df2c5dacb4c659f2488d80b90
       1044a25d94a00000000000000000000000000000000000000000000000022b1c8c1227a0000
       000000000000000000000000000000000000000000000003f0a59430f92a924400000000000
       000000000000000000000000000000000000000000000000000a00000000000000000000000
       0012021043bbaab3b71b2217655787a13d24cf618b000000000000000000000000000000000
       00000000000000000000000603c6a1e00000000000000000000000000000000000000000000
       00000000000000000002000000000000000000000000fe9a29ab92522d14fc65880d8172142
       61d8479ae000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc225
       a01df6c364ee7d2b684bbb6e3892fee69a1bc4fc487222b003ea57ec1596884916a01e1643f
       de193fde5e6be4ae0b2d4c4669560132a6dc87b6404d5c0cdc743fee6
    "
        );

        let transaction = SignedTransaction::try_from(&raw[..]).unwrap();

        // test sender recovery
        assert_eq!(
            H160::from_slice(&hex!("12021043bbaab3b71b2217655787a13d24cf618b")),
            transaction.sender_address().unwrap()
        );

        // test transaction hash computation:
        assert_eq!(
            H256::from_slice(&hex!(
                "3741aea434dc6e9e740be0113af4bac372fcdd2fa2188409c93c9405cbdcaaf0"
            )),
            transaction.hash()
        );

        // test decoded fields
        if let Transaction::Legacy { chain_id, nonce, gas_price, gas_limit, action, value, input } =
            transaction.transaction
        {
            assert_eq!(Some(1), chain_id);
            assert_eq!(8, nonce);
            assert_eq!(U256::from(74000001459u64), gas_price);
            assert_eq!(166236, gas_limit);
            assert_eq!(U256::zero(), value);

            assert_eq!(
                TransactionAction::Call(H160::from_slice(&hex!(
                    "7a250d5630b4cf539739df2c5dacb4c659f2488d"
                ))),
                action
            );

            assert_eq!(
                &hex!(
                    "4a25d94a00000000000000000000000000000000000000000000000022b1c8c1227a
           0000000000000000000000000000000000000000000000000003f0a59430f92a9244
           00000000000000000000000000000000000000000000000000000000000000a00000
           0000000000000000000012021043bbaab3b71b2217655787a13d24cf618b00000000
           000000000000000000000000000000000000000000000000603c6a1e000000000000
           00000000000000000000000000000000000000000000000000020000000000000000
           00000000fe9a29ab92522d14fc65880d817214261d8479ae00000000000000000000
           0000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
                ) as &[u8],
                &input
            );
        } else {
            panic!("decoded into wrong transaction type");
        }
    }

    #[test]
    fn decode_eip2930_transaction() {
        // https://etherscan.io/tx/0xfbf20efe99271206c0f5b497a92bee2e66f8bf9991e07648935194f17610b36e
        let raw = hex!(
            "01f8bb01808522ecb25c008307a120942a48420d75777af4c99970c0ed3c25effd1c08b
      e80843ccfd60bf84ff794fbfed54d426217bf75d2ce86622c1e5faf16b0a6e1a00000000
      000000000000000000000000000000000000000000000000000000000d694d9db270c1b5
      e3bd161e8c8503c55ceabee709552c080a03057d1077af1fc48bdfe2a8eac03caf686145
      b52342e77ad6982566fe39e0691a00507044aa767a50dc926d0daa4dd616b1e5a8d2e578
      1df5bc9feeee5a5139d61"
        );

        let transaction = SignedTransaction::try_from(&raw[..]).unwrap();

        // test if the right transaction type was detected
        assert!(matches!(transaction.transaction, Transaction::EIP2930 { .. }));

        // test transaction hash computation:
        assert_eq!(
            H256::from_slice(&hex!(
                "fbf20efe99271206c0f5b497a92bee2e66f8bf9991e07648935194f17610b36e"
            )),
            transaction.hash()
        );

        // test sender recovery
        assert_eq!(
            H160::from_slice(&hex!("4e2b6cc39e22026d8ce21214646a657ab7eb92b3")),
            transaction.sender_address().unwrap()
        );

        if let Transaction::EIP2930 {
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            action,
            value,
            input,
            access_list,
        } = transaction.transaction
        {
            assert_eq!(1, chain_id);
            assert_eq!(0, nonce);
            assert_eq!(U256::from(150000000000u64), gas_price);
            assert_eq!(500000, gas_limit);
            assert_eq!(U256::zero(), value);
            assert_eq!(
                TransactionAction::Call(H160::from_slice(&hex!(
                    "2a48420d75777af4c99970c0ed3c25effd1c08be"
                ))),
                action
            );
            assert_eq!(&hex!("3ccfd60b") as &[u8], &input);
            assert_eq!(
                vec![
                    AccessListItem {
                        address: H160::from_slice(&hex!(
                            "fbfed54d426217bf75d2ce86622c1e5faf16b0a6"
                        )),
                        slots: vec![H256::from_slice(&hex!(
                            "0000000000000000000000000000000000000000000000000000000000000000"
                        ))]
                    },
                    AccessListItem {
                        address: H160::from_slice(&hex!(
                            "d9db270c1b5e3bd161e8c8503c55ceabee709552"
                        )),
                        slots: vec![]
                    }
                ],
                access_list
            )
        } else {
            panic!("decoded into wrong transaction type");
        }
    }

    #[test]
    fn decode_eip1559_transaction() {
        // https://etherscan.io/tx/0x734678f719001015c5b5f5cbac6a9210ede7ee6ce63e746ff2e9eecda3ab68c7
        let raw = hex!(
            "02f8720104843b9aca008504eb6480bc82520894f76c5b19e86c256
       482f4aad1dae620a0c3ac0cd68717699d954d540080c080a05a5206a8e0486b8e101bcf
       4ed5b290df24a4d54f1ca752c859fa19c291244b98a0177166d96fd69db70628d99855b
       400c8a149b2254c211a0a00645830f5338218"
        );

        let transaction = SignedTransaction::try_from(&raw[..]).unwrap();

        // test if the right transaction type was detected
        assert!(matches!(transaction.transaction, Transaction::EIP1559 { .. }));

        // test transaction hash computation:
        assert_eq!(
            H256::from_slice(&hex!(
                "734678f719001015c5b5f5cbac6a9210ede7ee6ce63e746ff2e9eecda3ab68c7"
            )),
            transaction.hash()
        );

        // test sender recovery
        assert_eq!(
            H160::from_slice(&hex!("d882fab949fe224befd0e85afcc5f13d67980102")),
            transaction.sender_address().unwrap()
        );

        if let Transaction::EIP1559 {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            action,
            value,
            input,
            access_list,
        } = transaction.transaction
        {
            assert_eq!(1, chain_id);
            assert_eq!(4, nonce);
            assert_eq!(21000, gas_limit);
            assert_eq!(U256::from(6590050000000000u64), value);
            assert_eq!(U256::from(1000000000), max_priority_fee_per_gas);
            assert_eq!(U256::from(21129101500u64), max_fee_per_gas);
            assert_eq!(&[] as &[u8], &input);
            assert_eq!(Vec::<AccessListItem>::new(), access_list);
            assert_eq!(
                TransactionAction::Call(H160::from_slice(&hex!(
                    "f76c5b19e86c256482f4aad1dae620a0c3ac0cd6"
                ))),
                action
            );
        } else {
            panic!("decoded into wrong transaction type");
        }
    }
}
