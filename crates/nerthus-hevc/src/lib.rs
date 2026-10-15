//! An HEVC (H.265) decoder for still pictures: a port of libde265 1.0.16's
//! decoding path for intra slices, bit-exact with it, including its loop
//! filters and its handling of broken streams where that is defined. Its
//! NAL queue, picture order counts and reorder buffer follow libde265 too,
//! so a stream's first output picture comes out after the same input;
//! inter slices are passed over without decoding.

// The loops follow libde265's indexing.
#![allow(clippy::needless_range_loop)]

mod bitreader;
mod cabac;
mod deblock;
mod decoder;
mod error;
mod intra;
mod params;
mod picture;
mod sao;
mod scan;
mod slice;
mod slice_header;
mod transform;
mod util;

pub use decoder::{decode_nal_units, DecodedPicture, Decoder};
pub use error::Error;
