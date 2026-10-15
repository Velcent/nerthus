//! libde265's error and warning codes (`de265_error`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

impl Error {
    pub const OK: Error = Error(0);
    pub const COEFFICIENT_OUT_OF_IMAGE_BOUNDS: Error = Error(4);
    pub const CHECKSUM_MISMATCH: Error = Error(5);
    pub const CTB_OUTSIDE_IMAGE_AREA: Error = Error(6);
    pub const OUT_OF_MEMORY: Error = Error(7);
    pub const CODED_PARAMETER_OUT_OF_RANGE: Error = Error(8);
    pub const IMAGE_BUFFER_FULL: Error = Error(9);
    pub const WAITING_FOR_INPUT_DATA: Error = Error(13);
    pub const CANNOT_PROCESS_SEI: Error = Error(14);
    pub const PARAMETER_PARSING: Error = Error(15);
    pub const NO_INITIAL_SLICE_HEADER: Error = Error(16);
    pub const PREMATURE_END_OF_SLICE: Error = Error(17);
    pub const UNSPECIFIED_DECODING_ERROR: Error = Error(18);
    pub const NOT_IMPLEMENTED_YET: Error = Error(502);
    pub const WARNING_PREMATURE_END_OF_SLICE_SEGMENT: Error = Error(1002);
    pub const WARNING_INCORRECT_ENTRY_POINT_OFFSET: Error = Error(1003);
    pub const WARNING_CTB_OUTSIDE_IMAGE_AREA: Error = Error(1004);
    pub const WARNING_SPS_HEADER_INVALID: Error = Error(1005);
    pub const WARNING_PPS_HEADER_INVALID: Error = Error(1006);
    pub const WARNING_SLICEHEADER_INVALID: Error = Error(1007);
    pub const WARNING_NONEXISTING_PPS_REFERENCED: Error = Error(1009);
    pub const WARNING_NONEXISTING_SPS_REFERENCED: Error = Error(1010);
    pub const WARNING_NONEXISTING_REFERENCE_PICTURE_ACCESSED: Error = Error(1012);
    pub const WARNING_NUMBER_OF_SHORT_TERM_REF_PIC_SETS_OUT_OF_RANGE: Error = Error(1014);
    pub const WARNING_SHORT_TERM_REF_PIC_SET_OUT_OF_RANGE: Error = Error(1015);
    pub const WARNING_FAULTY_REFERENCE_PICTURE_LIST: Error = Error(1016);
    pub const WARNING_EOSS_BIT_NOT_SET: Error = Error(1017);
    pub const WARNING_MAX_NUM_REF_PICS_EXCEEDED: Error = Error(1018);
    pub const WARNING_INVALID_CHROMA_FORMAT: Error = Error(1019);
    pub const WARNING_SLICE_SEGMENT_ADDRESS_INVALID: Error = Error(1020);
    pub const WARNING_DEPENDENT_SLICE_WITH_ADDRESS_ZERO: Error = Error(1021);
    pub const NON_EXISTING_LT_REFERENCE_CANDIDATE_IN_SLICE_HEADER: Error = Error(1023);
    pub const WARNING_PCM_BITDEPTH_TOO_LARGE: Error = Error(1027);
    pub const WARNING_CHROMA_OF_CURRENT_IMAGE_DOES_NOT_MATCH_SPS: Error = Error(1030);
    pub const WARNING_BIT_DEPTH_OF_CURRENT_IMAGE_DOES_NOT_MATCH_SPS: Error = Error(1031);
    pub const WARNING_INVALID_SLICE_HEADER_INDEX_ACCESS: Error = Error(1033);

    /// `de265_isOK`: success or a warning.
    pub fn is_ok(self) -> bool {
        self.0 == 0 || self.0 >= 1000
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "HEVC decoding error {}", self.0)
    }
}

impl std::error::Error for Error {}
