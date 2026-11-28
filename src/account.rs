use std::fmt;

use anyhow::bail;
use blake2::{Blake2b512, Digest};
use subxt::utils::eth::AccountId20;

/// An account of the chain: 20 bytes on a chain with Ethereum-style accounts, 32 on the others.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Account {
    Id20([u8; 20]),
    Id32([u8; 32]),
}

impl Account {
    /// Shows 20 bytes as H160 with the EIP-55 checksum and 32 bytes in SS58 with `ss58_prefix`.
    pub fn display(&self, ss58_prefix: u16) -> Display<'_> {
        Display { account: self, ss58_prefix }
    }
}

impl TryFrom<&[u8]> for Account {
    type Error = anyhow::Error;

    fn try_from(bytes: &[u8]) -> anyhow::Result<Self> {
        if let Ok(id) = <[u8; 20]>::try_from(bytes) {
            Ok(Self::Id20(id))
        } else if let Ok(id) = <[u8; 32]>::try_from(bytes) {
            Ok(Self::Id32(id))
        } else {
            bail!("an account is 20 or 32 bytes, not {}", bytes.len())
        }
    }
}

pub struct Display<'a> {
    account: &'a Account,
    ss58_prefix: u16,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.account {
            Account::Id20(id) => write!(f, "{}", AccountId20(*id)),
            Account::Id32(id) => f.write_str(&ss58(id, self.ss58_prefix)),
        }
    }
}

/// SS58 has room for 14 bits of prefix, so the two highest are dropped, as in sp-core's
/// [`to_ss58check_with_version`](https://github.com/paritytech/polkadot-sdk/blob/0f50c61a952e6ec776df557d8ffd54f341bd8b6a/substrate/primitives/core/src/crypto.rs#L330-L349).
fn ss58(id: &[u8; 32], prefix: u16) -> String {
    let prefix = prefix & 0x3fff;
    let [lower, upper] = prefix.to_le_bytes();
    let mut data = if prefix < 64 {
        vec![lower]
    } else {
        vec![0b0100_0000 | (lower >> 2), (lower << 6) | upper]
    };
    data.extend_from_slice(id);
    let hash = Blake2b512::new()
        .chain_update(b"SS58PRE")
        .chain_update(&data)
        .finalize();
    data.extend_from_slice(&hash[..2]);

    bs58::encode(data).into_string()
}

#[cfg(test)]
mod tests {
    use super::Account;

    /// The public key of //Alice.
    const ALICE: &str = "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";

    fn from_hex(account: &str) -> Account {
        Account::try_from(hex::decode(account).unwrap().as_slice()).unwrap()
    }

    #[test]
    fn account_is_20_or_32_bytes() {
        assert_eq!(
            Account::try_from(&[1; 20][..]).unwrap(),
            Account::Id20([1; 20])
        );
        assert_eq!(
            Account::try_from(&[1; 32][..]).unwrap(),
            Account::Id32([1; 32])
        );
        assert!(Account::try_from(&[1; 33][..]).is_err());
    }

    #[test]
    fn account_of_32_bytes_shows_in_ss58_with_the_prefix() {
        // The addresses that subkey 3.0.0 prints for //Alice.
        let addresses = [
            (0, "15oF4uVJwmo4TdGW7VfQxNLavjCXviqxT9S1MgbjMNHr6Sp5"),
            (2, "HNZata7iMYWmk5RvZRTiAsSDhV8366zq2YGb3tLH5Upf74F"),
            (63, "7NPoMQbiA6trJKkjB35uk96MeJD4PGWkLQLH7k7hXEkZpiba"),
            (65, "cLxkfNUiCYsb57YLhTJdNVKxUTB1VTpeygYZNhYuFc83KrFy7"),
            (7391, "unjKJQJrRd238pkUZZvzDQrfKuM39zBSnQ5zjAGAGcdRhaJTx"),
            (13116, "bt7zDqc4kAcKZ6kWup5yyuvRAbeYPhXGxwXY4CguG476A9MCE"),
        ];

        for (prefix, address) in addresses {
            assert_eq!(from_hex(ALICE).display(prefix).to_string(), address);
        }
    }

    #[test]
    fn prefix_wider_than_14_bits_loses_its_two_highest_bits() {
        let alice = from_hex(ALICE);

        assert_eq!(
            alice.display(0xc000 | 42).to_string(),
            alice.display(42).to_string()
        );
    }

    #[test]
    fn account_of_20_bytes_shows_as_h160_with_its_checksum() {
        // An example from EIP-55.
        let account = from_hex("5aaeb6053f3e94c9b9a09f33669435e7ef1beaed");

        assert_eq!(
            account.display(0).to_string(),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );
    }
}
