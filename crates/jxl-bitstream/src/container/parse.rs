use std::borrow::Cow;

use super::box_header::*;
use super::{BitstreamKind, ContainerParser, DetectState, JxlpIndexState, OooJxlpPayload};
use crate::consts;
use crate::error::{BitstreamResult, Error};

/// Iterator that reads over a buffer and emits parser events.
pub struct ParseEvents<'inner, 'buf> {
    inner: &'inner mut ContainerParser,
    remaining_input: &'buf [u8],
    finished: bool,
}

impl<'inner, 'buf> ParseEvents<'inner, 'buf> {
    pub(super) fn new(parser: &'inner mut ContainerParser, input: &'buf [u8]) -> Self {
        parser.previous_consumed_bytes = 0;
        Self {
            inner: parser,
            remaining_input: input,
            finished: false,
        }
    }

    fn emit_single(&mut self) -> BitstreamResult<Option<ParseEvent<'buf>>> {
        let state = &mut self.inner.state;
        let jxlp_index_state = &mut self.inner.jxlp_index_state;
        let file_format_version = &mut self.inner.file_format_version;
        let ftyp_header = &mut self.inner.ftyp_header;
        let ftyp_header_len = &mut self.inner.ftyp_header_len;
        let jxlp_ooo_buffer = &mut self.inner.jxlp_ooo_buffer;
        let buf = &mut self.remaining_input;

        loop {
            // Out-of-order jxlp payloads buffered while their predecessors were
            // missing become emit-able once the missing pieces have arrived.
            // Injection only happens at box boundaries: the in-order piece that
            // advanced the index must emit its payload first.
            if matches!(*state, DetectState::WaitingBoxHeader)
                && let JxlpIndexState::Jxlp(expected_index) = jxlp_index_state
                && let Some(payload) = jxlp_ooo_buffer.remove(expected_index)
            {
                *expected_index += 1;
                if payload.is_last {
                    *jxlp_index_state = JxlpIndexState::JxlpFinished;
                }
                return Ok(Some(ParseEvent::Codestream(Cow::Owned(payload.data))));
            }

            if buf.is_empty() {
                self.finished = true;
                return Ok(None);
            }

            match state {
                DetectState::WaitingSignature => {
                    if buf.starts_with(&consts::CODESTREAM_SIG) {
                        tracing::trace!("Codestream signature found");
                        *state = DetectState::InCodestream {
                            kind: BitstreamKind::BareCodestream,
                            bytes_left: None,
                            pending_no_more_aux_box: true,
                        };
                        return Ok(Some(ParseEvent::BitstreamKind(
                            BitstreamKind::BareCodestream,
                        )));
                    } else if buf.starts_with(&consts::CONTAINER_SIG) {
                        tracing::trace!("Container signature found");
                        *state = DetectState::WaitingBoxHeader;
                        *buf = &buf[consts::CONTAINER_SIG.len()..];
                        return Ok(Some(ParseEvent::BitstreamKind(BitstreamKind::Container)));
                    } else if !consts::CODESTREAM_SIG.starts_with(buf)
                        && !consts::CONTAINER_SIG.starts_with(buf)
                    {
                        tracing::debug!(?buf, "Invalid signature");
                        *state = DetectState::InCodestream {
                            kind: BitstreamKind::Invalid,
                            bytes_left: None,
                            pending_no_more_aux_box: true,
                        };
                        return Ok(Some(ParseEvent::BitstreamKind(BitstreamKind::Invalid)));
                    } else {
                        return Ok(None);
                    }
                }

                DetectState::WaitingBoxHeader => match ContainerBoxHeader::parse(buf)? {
                    HeaderParseResult::Done {
                        header,
                        header_size,
                    } => {
                        *buf = &buf[header_size..];
                        let tbox = header.box_type();
                        if tbox == ContainerBoxType::CODESTREAM {
                            match jxlp_index_state {
                                JxlpIndexState::Initial => {
                                    *jxlp_index_state = JxlpIndexState::SingleJxlc;
                                }
                                JxlpIndexState::SingleJxlc => {
                                    tracing::debug!("Duplicate jxlc box found");
                                    return Err(Error::InvalidBox);
                                }
                                JxlpIndexState::Jxlp(_) | JxlpIndexState::JxlpFinished => {
                                    tracing::debug!("Found jxlc box instead of jxlp box");
                                    return Err(Error::InvalidBox);
                                }
                            }

                            let bytes_left = header.box_size().map(|x| x as usize);
                            *state = DetectState::InCodestream {
                                kind: BitstreamKind::Container,
                                bytes_left,
                                pending_no_more_aux_box: bytes_left.is_none(),
                            };
                        } else if tbox == ContainerBoxType::PARTIAL_CODESTREAM {
                            if let Some(box_size) = header.box_size()
                                && box_size < 4
                            {
                                return Err(Error::InvalidBox);
                            }

                            match jxlp_index_state {
                                JxlpIndexState::Initial => {
                                    *jxlp_index_state = JxlpIndexState::Jxlp(0);
                                }
                                JxlpIndexState::Jxlp(_) => {}
                                JxlpIndexState::SingleJxlc => {
                                    tracing::debug!("jxlp box found after jxlc box");
                                    return Err(Error::InvalidBox);
                                }
                                JxlpIndexState::JxlpFinished => {
                                    tracing::debug!("found another jxlp box after the final one");
                                    return Err(Error::InvalidBox);
                                }
                            }

                            *state = DetectState::WaitingJxlpIndex(header);
                        } else {
                            let bytes_left = header.box_size().map(|x| x as usize);
                            let ty = header.box_type();
                            let mut brotli_compressed = ty == ContainerBoxType::BROTLI_COMPRESSED;
                            if brotli_compressed {
                                if let Some(0..=3) = bytes_left {
                                    tracing::error!(
                                        bytes_left = bytes_left.unwrap(),
                                        "Brotli-compressed box is too small"
                                    );
                                    return Err(Error::InvalidBox);
                                }
                                brotli_compressed = true;
                            }

                            *state = DetectState::InAuxBox {
                                header,
                                brotli_box_type: None,
                                bytes_left,
                            };

                            if !brotli_compressed {
                                return Ok(Some(ParseEvent::AuxBoxStart {
                                    ty,
                                    brotli_compressed: false,
                                    last_box: bytes_left.is_none(),
                                }));
                            }
                        }
                    }
                    HeaderParseResult::NeedMoreData => return Ok(None),
                },

                DetectState::WaitingJxlpIndex(header) => {
                    let &[b0, b1, b2, b3, ..] = &**buf else {
                        return Ok(None);
                    };

                    let index = u32::from_be_bytes([b0, b1, b2, b3]);
                    *buf = &buf[4..];
                    let is_last = index & 0x80000000 != 0;
                    let index = index & 0x7fffffff;

                    let JxlpIndexState::Jxlp(expected_index) = *jxlp_index_state else {
                        unreachable!("invalid jxlp index state in WaitingJxlpIndex");
                    };
                    let bytes_left = header.box_size().map(|x| x as usize - 4);

                    if index == expected_index {
                        *jxlp_index_state = if is_last {
                            JxlpIndexState::JxlpFinished
                        } else {
                            JxlpIndexState::Jxlp(expected_index + 1)
                        };
                        *state = DetectState::InCodestream {
                            kind: BitstreamKind::Container,
                            bytes_left,
                            pending_no_more_aux_box: bytes_left.is_none(),
                        };
                    } else if index < expected_index {
                        tracing::debug!(index, "Duplicate jxlp box found");
                        return Err(Error::InvalidBox);
                    } else if *file_format_version >= 1 {
                        // ftyp minor version 1 permits out-of-order jxlp
                        // boxes; buffer the payload until the boxes before it
                        // have been seen (same as libjxl).
                        if jxlp_ooo_buffer.contains_key(&index) {
                            tracing::debug!(index, "Duplicate jxlp box index found");
                            return Err(Error::InvalidBox);
                        }
                        *state = DetectState::BufferingJxlp {
                            index,
                            is_last,
                            bytes_left,
                            payload: Vec::new(),
                        };
                    } else {
                        tracing::debug!(
                            expected_index,
                            actual_index = index,
                            "Out-of-order jxlp box found",
                        );
                        return Err(Error::InvalidBox);
                    }
                }

                DetectState::BufferingJxlp {
                    bytes_left,
                    payload,
                    ..
                } => {
                    let take = bytes_left.map_or(buf.len(), |n| n.min(buf.len()));
                    payload.extend_from_slice(&buf[..take]);
                    *buf = &buf[take..];
                    if let Some(n) = bytes_left {
                        *n -= take;
                        if *n == 0 {
                            let DetectState::BufferingJxlp {
                                index,
                                is_last,
                                payload,
                                ..
                            } = std::mem::replace(state, DetectState::WaitingBoxHeader)
                            else {
                                unreachable!()
                            };
                            jxlp_ooo_buffer.insert(
                                index,
                                OooJxlpPayload {
                                    data: payload,
                                    is_last,
                                },
                            );
                        }
                    }
                }

                // JXL codestream box is the last box; emit "no more aux box" event.
                DetectState::InCodestream {
                    pending_no_more_aux_box: pending @ true,
                    ..
                } => {
                    *pending = false;
                    return Ok(Some(ParseEvent::NoMoreAuxBox));
                }

                DetectState::InCodestream {
                    bytes_left: None, ..
                } => {
                    let payload = *buf;
                    *buf = &[];
                    return Ok(Some(ParseEvent::Codestream(Cow::Borrowed(payload))));
                }
                DetectState::InCodestream {
                    bytes_left: Some(bytes_left),
                    ..
                } => {
                    let payload = if buf.len() >= *bytes_left {
                        let (payload, remaining) = buf.split_at(*bytes_left);
                        *state = DetectState::WaitingBoxHeader;
                        *buf = remaining;
                        payload
                    } else {
                        let payload = *buf;
                        *bytes_left -= buf.len();
                        *buf = &[];
                        payload
                    };
                    return Ok(Some(ParseEvent::Codestream(Cow::Borrowed(payload))));
                }

                // Read brob payload box type.
                DetectState::InAuxBox {
                    header:
                        ContainerBoxHeader {
                            ty: ContainerBoxType::BROTLI_COMPRESSED,
                            ..
                        },
                    brotli_box_type: brotli_box_type @ None,
                    bytes_left,
                } => {
                    if buf.len() < 4 {
                        return Ok(None);
                    }

                    let (ty_slice, remaining) = buf.split_at(4);
                    *buf = remaining;
                    if let Some(bytes_left) = bytes_left {
                        *bytes_left -= 4;
                    }

                    let mut ty = [0u8; 4];
                    ty.copy_from_slice(ty_slice);
                    let is_reserved_box_type =
                        &ty[..3] == b"jxl" || &ty == b"brob" || &ty == b"jbrd";
                    if is_reserved_box_type {
                        return Err(Error::ValidationFailed(
                            "brob box, jxl boxes and jbrd box cannot be Brotli-compressed",
                        ));
                    }

                    let ty = ContainerBoxType(ty);
                    *brotli_box_type = Some(ty);

                    return Ok(Some(ParseEvent::AuxBoxStart {
                        ty,
                        brotli_compressed: true,
                        last_box: bytes_left.is_none(),
                    }));
                }

                DetectState::InAuxBox {
                    header: ContainerBoxHeader { ty: outer_ty, .. },
                    brotli_box_type,
                    bytes_left,
                } => {
                    let outer_ty = *outer_ty;
                    let ty = if let Some(ty) = brotli_box_type {
                        *ty
                    } else {
                        outer_ty
                    };

                    let payload = match bytes_left {
                        Some(0) => {
                            *state = DetectState::WaitingBoxHeader;
                            if outer_ty == ContainerBoxType::FILE_TYPE && *ftyp_header_len < 8 {
                                tracing::debug!("ftyp box too small");
                                return Err(Error::InvalidBox);
                            }
                            return Ok(Some(ParseEvent::AuxBoxEnd(ty)));
                        }
                        Some(bytes_left) => {
                            let num_bytes_to_read = (*bytes_left).min(buf.len());
                            let (payload, remaining) = buf.split_at(num_bytes_to_read);
                            *bytes_left -= num_bytes_to_read;
                            *buf = remaining;
                            payload
                        }
                        None => {
                            let payload = *buf;
                            *buf = &[];
                            payload
                        }
                    };

                    if outer_ty == ContainerBoxType::FILE_TYPE && *ftyp_header_len < 8 {
                        let take = (8 - *ftyp_header_len).min(payload.len());
                        ftyp_header[*ftyp_header_len..*ftyp_header_len + take]
                            .copy_from_slice(&payload[..take]);
                        *ftyp_header_len += take;
                        if *ftyp_header_len == 8 {
                            let version = u32::from_be_bytes(ftyp_header[4..8].try_into().unwrap());
                            if &ftyp_header[..4] != b"jxl " || version > 1 {
                                tracing::debug!("Invalid ftyp box contents");
                                return Err(Error::InvalidBox);
                            }
                            *file_format_version = version;
                        }
                    }

                    return Ok(Some(ParseEvent::AuxBoxData(ty, payload)));
                }
            }
        }
    }
}

impl std::fmt::Debug for ParseEvents<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseEvents")
            .field("inner", &self.inner)
            .field(
                "remaining_input",
                &format_args!("({} byte(s))", self.remaining_input.len()),
            )
            .field("finished", &self.finished)
            .finish()
    }
}

impl<'buf> Iterator for ParseEvents<'_, 'buf> {
    type Item = BitstreamResult<ParseEvent<'buf>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let initial_buf = self.remaining_input;
        let event = self.emit_single();

        if event.is_err() {
            self.finished = true;
        }

        self.inner.previous_consumed_bytes += initial_buf.len() - self.remaining_input.len();
        event.transpose()
    }
}

/// Parser event emitted by [`ParseEvents`].
pub enum ParseEvent<'buf> {
    /// Bitstream structure is detected.
    BitstreamKind(BitstreamKind),
    /// Codestream data is read.
    ///
    /// Returned data may be partial. Complete codestream can be obtained by concatenating all data
    /// of `Codestream` events. Buffered out-of-order `jxlp` payloads are owned.
    Codestream(Cow<'buf, [u8]>),
    /// No more auxiliary box will come after this event.
    NoMoreAuxBox,
    /// New auxiliary box starts.
    AuxBoxStart {
        /// Type of this box.
        ty: ContainerBoxType,
        /// Indicates whether the payload is Brotli-compressed.
        brotli_compressed: bool,
        /// Indicates whether this box is the last box of the bitstream.
        last_box: bool,
    },
    /// Auxiliary box data is read.
    ///
    /// Returned data may be partial. Complete box payload can be obtained by concatenating all
    /// data between corresponding `AuxBoxStart` and `AuxBoxEnd` events.
    AuxBoxData(ContainerBoxType, &'buf [u8]),
    /// Current auxiliary box has ended.
    AuxBoxEnd(ContainerBoxType),
}

impl std::fmt::Debug for ParseEvent<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BitstreamKind(kind) => f.debug_tuple("BitstreamKind").field(kind).finish(),
            Self::Codestream(buf) => f
                .debug_tuple("Codestream")
                .field(&format_args!("{} byte(s)", buf.len()))
                .finish(),
            Self::NoMoreAuxBox => write!(f, "NoMoreAuxBox"),
            Self::AuxBoxStart {
                ty,
                brotli_compressed,
                last_box,
            } => f
                .debug_struct("AuxBoxStart")
                .field("ty", ty)
                .field("brotli_compressed", brotli_compressed)
                .field("last_box", last_box)
                .finish(),
            Self::AuxBoxData(ty, buf) => f
                .debug_tuple("AuxBoxData")
                .field(ty)
                .field(&format_args!("{} byte(s)", buf.len()))
                .finish(),
            Self::AuxBoxEnd(ty) => f.debug_tuple("AuxBoxEnd").field(&ty).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::CONTAINER_SIG;

    fn bx(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(ty);
        v.extend_from_slice(payload);
        v
    }

    fn ftyp(version: u32) -> Vec<u8> {
        let mut payload = b"jxl ".to_vec();
        payload.extend_from_slice(&version.to_be_bytes());
        payload.extend_from_slice(b"jxlc");
        bx(b"ftyp", &payload)
    }

    fn jxlp(index: u32, is_last: bool, payload: &[u8]) -> Vec<u8> {
        let tag = index | ((is_last as u32) << 31);
        let mut inner = tag.to_be_bytes().to_vec();
        inner.extend_from_slice(payload);
        bx(b"jxlp", &inner)
    }

    fn container(version: u32, boxes: &[Vec<u8>]) -> Vec<u8> {
        let mut v = CONTAINER_SIG.to_vec();
        v.extend_from_slice(&ftyp(version));
        for b in boxes {
            v.extend_from_slice(b);
        }
        v
    }

    fn collect_codestream(data: &[u8], chunk_size: usize) -> BitstreamResult<Vec<u8>> {
        let mut parser = ContainerParser::new();
        let mut out = Vec::new();
        let mut pending = Vec::new();
        for chunk in data.chunks(chunk_size.max(1)) {
            pending.extend_from_slice(chunk);
            for event in parser.feed_bytes(&pending) {
                if let ParseEvent::Codestream(payload) = event? {
                    out.extend_from_slice(&payload);
                }
            }
            pending.drain(..parser.previous_consumed_bytes());
        }
        for event in parser.feed_bytes(&[]) {
            if let ParseEvent::Codestream(payload) = event? {
                out.extend_from_slice(&payload);
            }
        }
        Ok(out)
    }

    #[test]
    fn jxlp_in_order() {
        for version in [0u32, 1] {
            let data = container(
                version,
                &[
                    jxlp(0, false, b"AAA"),
                    jxlp(1, false, b"BBB"),
                    jxlp(2, true, b"CCC"),
                ],
            );
            assert_eq!(collect_codestream(&data, usize::MAX).unwrap(), b"AAABBBCCC");
        }
    }

    #[test]
    fn jxlp_out_of_order_minor_version_1() {
        let data = container(
            1,
            &[
                jxlp(0, false, b"AAA"),
                jxlp(2, true, b"CCC"),
                jxlp(1, false, b"BBB"),
            ],
        );
        assert_eq!(collect_codestream(&data, usize::MAX).unwrap(), b"AAABBBCCC");
    }

    #[test]
    fn jxlp_out_of_order_minor_version_0_rejected() {
        let data = container(
            0,
            &[
                jxlp(0, false, b"AAA"),
                jxlp(2, true, b"CCC"),
                jxlp(1, false, b"BBB"),
            ],
        );
        assert!(collect_codestream(&data, usize::MAX).is_err());
    }

    #[test]
    fn jxlp_out_of_order_no_ftyp_rejected() {
        let mut data = CONTAINER_SIG.to_vec();
        data.extend_from_slice(&jxlp(1, true, b"BBB"));
        data.extend_from_slice(&jxlp(0, false, b"AAA"));
        assert!(collect_codestream(&data, usize::MAX).is_err());
    }

    #[test]
    fn jxlp_duplicate_index_rejected() {
        // index 0 twice
        let data = container(1, &[jxlp(0, false, b"AAA"), jxlp(0, false, b"aaa")]);
        assert!(collect_codestream(&data, usize::MAX).is_err());

        // duplicated buffered index
        let data = container(
            1,
            &[
                jxlp(0, false, b"AAA"),
                jxlp(2, false, b"CCC"),
                jxlp(2, true, b"ccc"),
            ],
        );
        assert!(collect_codestream(&data, usize::MAX).is_err());
    }

    #[test]
    fn jxlp_after_last_rejected() {
        let data = container(1, &[jxlp(0, true, b"AAA"), jxlp(1, true, b"BBB")]);
        assert!(collect_codestream(&data, usize::MAX).is_err());
    }

    #[test]
    fn ftyp_invalid_contents_rejected() {
        // bad major brand
        let mut data = CONTAINER_SIG.to_vec();
        data.extend_from_slice(&bx(b"ftyp", b"exif\x00\x00\x00\x01jxlc"));
        data.extend_from_slice(&jxlp(0, true, b"AAA"));
        assert!(collect_codestream(&data, usize::MAX).is_err());

        // unsupported minor version
        let mut data = CONTAINER_SIG.to_vec();
        data.extend_from_slice(&ftyp(2));
        data.extend_from_slice(&jxlp(0, true, b"AAA"));
        assert!(collect_codestream(&data, usize::MAX).is_err());
    }

    #[test]
    fn jxlp_out_of_order_chunked() {
        let data = container(
            1,
            &[
                jxlp(0, false, b"AAA"),
                jxlp(2, true, b"CCC"),
                jxlp(1, false, b"BBB"),
            ],
        );
        for chunk_size in [1usize, 3, 7] {
            assert_eq!(collect_codestream(&data, chunk_size).unwrap(), b"AAABBBCCC");
        }
    }
}
