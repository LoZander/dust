use uuid::Uuid;

/// A message with a unique id to be sent over the p2p network.
///
/// Every [`Msg`] is guaranteed an upperbound in size.
/// It is guaranteed that the the message along with the UUID and a
/// seperating byte all together take up at most `CAPACITY` bytes.
///
/// When converted to bytes using [`Msg::into_bytes`], the resulting
/// array is padded with zeroes to take up exactly `CAPACITY` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub text: String,
    uuid: Uuid,
}

pub const UUID_SIZE: usize = 16;
pub const SEP_SIZE: usize = 1;
pub const CAPACITY: usize = 128;

#[derive(Debug, thiserror::Error)]
#[error("failed to convert `String` to `Msg`")]
pub struct TryFromStringToMsgError;

impl TryFrom<String> for Msg {
    type Error = TryFromStringToMsgError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let within_capacity = value.len() + SEP_SIZE + UUID_SIZE <= CAPACITY;

        let uuid = Uuid::new_v4();

        assert!(!uuid.as_bytes()[0] != 0, "Uuid started with 0!");

        if within_capacity {
            Ok(Self {
                text: value,
                uuid: Uuid::new_v4(),
            })
        } else {
            Err(TryFromStringToMsgError)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TryFromArrayToMsgError {
    #[error("missing seperator")]
    MissingSep,
    #[error("uuid error: `{0}`")]
    CorruptUuid(#[from] uuid::Error),
}

impl TryFrom<[u8; CAPACITY]> for Msg {
    type Error = TryFromArrayToMsgError;

    fn try_from(value: [u8; CAPACITY]) -> Result<Self, Self::Error> {
        let sep = value
            .iter()
            .position(|c| c == &0)
            .ok_or(TryFromArrayToMsgError::MissingSep)?;

        let text_bytes = &value[..sep];
        let uuid_bytes = &value[(sep + SEP_SIZE)..(sep + SEP_SIZE + UUID_SIZE)];

        let text = String::from_utf8_lossy(text_bytes).to_string();
        let uuid = Uuid::from_slice(uuid_bytes).unwrap();

        Ok(Self { text, uuid })
    }
}

impl Msg {
    /// Returns and array containing the message in bytes.
    ///
    /// The array contains both `text.msg` and `text.uuid`
    /// seperated with a `0` byte. The array has a fixed
    /// size of `CAPACITY` and is padded with trailing `0`s.
    pub fn into_bytes(self) -> [u8; CAPACITY] {
        let mut bytes = [0; CAPACITY];
        let length = self.text.len();
        self.text
            .into_bytes()
            .into_iter()
            .enumerate()
            .for_each(|(i, b)| bytes[i] = b);
        self.uuid
            .into_bytes()
            .into_iter()
            .enumerate()
            .for_each(|(i, b)| bytes[i + length + SEP_SIZE] = b);

        bytes
    }
}
