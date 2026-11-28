use std::fmt;

use anyhow::bail;

/// An account of the chain: 20 bytes on a chain with Ethereum-style accounts, 32 on the others.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Account {
    Id20([u8; 20]),
    Id32([u8; 32]),
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

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes: &[u8] = match self {
            Self::Id20(id) => id,
            Self::Id32(id) => id,
        };
        write!(f, "0x{}", hex::encode(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::Account;

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
}
