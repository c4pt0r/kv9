//! Shared authenticated point dispatch and lossless response envelope.
//! Stream and experiment transports project onto the same existing handlers.

use crate::grpc::{proto::kv9_server::Kv9, Authenticator, Kv9Grpc};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use tonic::{
    metadata::{KeyAndValueRef, MetadataMap},
    Request, Response, Status,
};

pub(crate) const FRAME_LIMIT: usize = crate::client::MAX_MESSAGE_BYTES + 8192;
pub(crate) const CONNECTION_LIMIT: usize = 8;
pub(crate) const CHANNEL_LIMIT: usize = crate::client::MAX_IN_FLIGHT;

#[derive(Serialize, Deserialize)]
pub(crate) struct WireMetadata {
    pub(crate) key: String,
    pub(crate) value: Vec<u8>,
    pub(crate) binary: bool,
}

pub(crate) fn encode_metadata(map: &MetadataMap) -> Result<Vec<WireMetadata>, Status> {
    map.iter()
        .map(|entry| match entry {
            KeyAndValueRef::Ascii(key, value) => Ok(WireMetadata {
                key: key.as_str().into(),
                value: value.as_encoded_bytes().to_vec(),
                binary: false,
            }),
            KeyAndValueRef::Binary(key, value) => Ok(WireMetadata {
                key: key.as_str().into(),
                value: value
                    .to_bytes()
                    .map_err(|_| Status::internal("invalid binary metadata"))?
                    .to_vec(),
                binary: true,
            }),
        })
        .collect()
}

pub(crate) fn decode_metadata(entries: Vec<WireMetadata>) -> Result<MetadataMap, Status> {
    let mut map = MetadataMap::new();
    for entry in entries {
        if entry.binary {
            let key = tonic::metadata::MetadataKey::<tonic::metadata::Binary>::from_bytes(
                entry.key.as_bytes(),
            )
            .map_err(|_| Status::data_loss("invalid binary metadata key"))?;
            map.append_bin(
                key,
                tonic::metadata::MetadataValue::from_bytes(&entry.value),
            );
        } else {
            let key = tonic::metadata::MetadataKey::<tonic::metadata::Ascii>::from_bytes(
                entry.key.as_bytes(),
            )
            .map_err(|_| Status::data_loss("invalid metadata key"))?;
            let value = tonic::metadata::MetadataValue::<tonic::metadata::Ascii>::try_from(
                entry.value.as_slice(),
            )
            .map_err(|_| Status::data_loss("invalid metadata value"))?;
            map.append(key, value);
        }
    }
    Ok(map)
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WireReply {
    pub(crate) code: i32,
    pub(crate) message: String,
    pub(crate) details: Vec<u8>,
    pub(crate) metadata: Vec<WireMetadata>,
    pub(crate) payload: Vec<u8>,
}

impl fmt::Debug for WireReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireReply")
            .field("code", &self.code)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

impl WireReply {
    pub(crate) fn encode<T: Message>(reply: Result<Response<T>, Status>) -> Self {
        match reply {
            Ok(response) if response.get_ref().encoded_len() > crate::client::MAX_MESSAGE_BYTES => {
                Self::error(Status::resource_exhausted("response exceeds message limit"))
            }
            Ok(response) => match encode_metadata(response.metadata()) {
                Ok(metadata) => Self {
                    code: 0,
                    message: String::new(),
                    details: Vec::new(),
                    metadata,
                    payload: response.into_inner().encode_to_vec(),
                },
                Err(error) => Self::error(error),
            },
            Err(error) => Self::error(error),
        }
    }
    pub(crate) fn error(error: Status) -> Self {
        match encode_metadata(error.metadata()) {
            Ok(metadata) => Self {
                code: error.code() as i32,
                message: error.message().into(),
                details: error.details().to_vec(),
                metadata,
                payload: Vec::new(),
            },
            Err(_) => Self {
                code: tonic::Code::Internal as i32,
                message: "invalid status metadata".into(),
                details: Vec::new(),
                metadata: Vec::new(),
                payload: Vec::new(),
            },
        }
    }
    pub(crate) fn decode<T: Message + Default>(self) -> Result<Response<T>, Status> {
        if self.payload.len() > crate::client::MAX_MESSAGE_BYTES || self.code < 0 || self.code > 16
        {
            return Err(Status::data_loss("invalid point response"));
        }
        let metadata = decode_metadata(self.metadata)?;
        if self.code != 0 {
            if !self.payload.is_empty() {
                return Err(Status::data_loss("error response contains success payload"));
            }
            return Err(Status::with_details_and_metadata(
                tonic::Code::from_i32(self.code),
                self.message,
                self.details.into(),
                metadata,
            ));
        }
        if !self.message.is_empty() || !self.details.is_empty() {
            return Err(Status::data_loss("success response contains error fields"));
        }
        let value = T::decode(self.payload.as_slice())
            .map_err(|_| Status::data_loss("invalid point response"))?;
        let mut response = Response::new(value);
        *response.metadata_mut() = metadata;
        Ok(response)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WireRequest {
    pub(crate) authorization: String,
    pub(crate) payload: Vec<u8>,
}

impl fmt::Debug for WireRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WireRequest")
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct Handler {
    pub(crate) api: Kv9Grpc,
    pub(crate) authenticator: Arc<dyn Authenticator>,
}

impl Handler {
    pub(crate) fn request<T: Message + Default>(
        &self,
        authorization: &str,
        payload: &[u8],
    ) -> Result<Request<T>, Status> {
        if authorization.len() > 4103 || payload.len() > crate::client::MAX_MESSAGE_BYTES {
            return Err(Status::resource_exhausted("frame exceeds point limits"));
        }
        let mut metadata = MetadataMap::new();
        metadata.insert(
            "authorization",
            authorization
                .parse()
                .map_err(|_| Status::unauthenticated("invalid authorization"))?,
        );
        let auth = self.authenticator.authenticate(&metadata)?;
        let message =
            T::decode(payload).map_err(|_| Status::invalid_argument("invalid point request"))?;
        let mut request = Request::new(message);
        *request.metadata_mut() = metadata;
        request.extensions_mut().insert(auth);
        Ok(request)
    }
}

impl Handler {
    pub(crate) async fn dispatch(self, operation: u8, request: WireRequest) -> WireReply {
        let WireRequest {
            authorization,
            payload,
        } = request;
        match operation {
            0 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_get(request).await,
                Err(error) => Err(error),
            }),
            1 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_put(request).await,
                Err(error) => Err(error),
            }),
            2 => WireReply::encode(match self.request(&authorization, &payload) {
                Ok(request) => self.api.raw_delete(request).await,
                Err(error) => Err(error),
            }),
            3 => WireReply::encode(
                match self.request::<crate::proto::RawBatchGetRequest>(&authorization, &payload) {
                    Ok(request) if crate::client::valid_batch_keys(&request.get_ref().keys) => {
                        self.api.raw_batch_get(request).await
                    }
                    Ok(_) => Err(Status::invalid_argument("invalid batch read bounds")),
                    Err(error) => Err(error),
                },
            ),
            4 => WireReply::encode(
                match self.request::<crate::proto::RawBatchPutRequest>(&authorization, &payload) {
                    Ok(request)
                        if (1..=crate::client::MAX_BATCH_ITEMS)
                            .contains(&request.get_ref().pairs.len())
                            && request.get_ref().pairs.iter().all(|pair| {
                                pair.key.len() <= crate::client::MAX_KEY_BYTES
                                    && pair.value.len() <= crate::client::MAX_VALUE_BYTES
                            }) =>
                    {
                        self.api.raw_batch_put(request).await
                    }
                    Ok(_) => Err(Status::invalid_argument("invalid batch write bounds")),
                    Err(error) => Err(error),
                },
            ),
            _ => WireReply::error(Status::unimplemented("unknown point operation")),
        }
    }
}
