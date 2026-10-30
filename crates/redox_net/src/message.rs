//! Typed messages: a `u16` id followed by a postcard body.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::NetError;

const HEADER_LEN: usize = 2;

/// A typed message. Ids must be unique across the game's message types.
pub trait Message: Serialize + DeserializeOwned {
    /// Wire id written in front of the body.
    const ID: u16;
}

/// Encodes `message` as `id (u16 LE) ++ postcard body`.
pub fn encode<M: Message>(message: &M) -> Result<Vec<u8>, NetError> {
    let body = postcard::to_stdvec(message).map_err(|error| NetError::Codec(error.to_string()))?;
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(&M::ID.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decodes a message of type `M`. Rejects other ids, short input and trailing bytes.
pub fn decode<M: Message>(bytes: &[u8]) -> Result<M, NetError> {
    let id = message_id(bytes)?;
    if id != M::ID {
        return Err(NetError::WrongMessageId {
            expected: M::ID,
            got: id,
        });
    }
    let body = bytes.get(HEADER_LEN..).unwrap_or(&[]);
    let (message, rest) =
        postcard::take_from_bytes::<M>(body).map_err(|error| NetError::Codec(error.to_string()))?;
    if !rest.is_empty() {
        return Err(NetError::Malformed("trailing bytes after message body"));
    }
    Ok(message)
}

/// Reads the id of an encoded message without decoding the body.
pub fn message_id(bytes: &[u8]) -> Result<u16, NetError> {
    let raw: [u8; HEADER_LEN] = bytes
        .get(..HEADER_LEN)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(NetError::Malformed("message shorter than its id"))?;
    Ok(u16::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::rng::SplitMix64;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Chat {
        text: String,
    }

    impl Message for Chat {
        const ID: u16 = 7;
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Other {
        text: String,
    }

    impl Message for Other {
        const ID: u16 = 8;
    }

    #[test]
    fn a_message_roundtrips_through_postcard_with_its_id() {
        let original = Chat {
            text: "hello".to_string(),
        };
        let bytes = encode(&original).expect("a small message encodes");
        assert_eq!(message_id(&bytes), Ok(7));
        assert_eq!(decode::<Chat>(&bytes), Ok(original));
    }

    #[test]
    fn a_message_with_another_id_is_rejected() {
        let bytes = encode(&Chat {
            text: "x".to_string(),
        })
        .expect("encodes");
        assert_eq!(
            decode::<Other>(&bytes),
            Err(NetError::WrongMessageId {
                expected: 8,
                got: 7
            })
        );
    }

    #[test]
    fn garbled_bytes_never_panic() {
        let mut rng = SplitMix64::new(11);
        for _ in 0..1000 {
            let len = (rng.next_u64() % 64) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
            let result = decode::<Chat>(&bytes);
            if bytes.len() < HEADER_LEN {
                assert!(result.is_err());
            }
        }
    }
}
