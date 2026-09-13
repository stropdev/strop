use super::*;

struct InterruptedWriter {
    remaining: usize,
    bytes: Vec<u8>,
}
impl Write for InterruptedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = self.remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn partial_write_keeps_capacity_charged_until_packet_is_released() {
    let mut queue = Writer::default();
    let mut payload = Vec::with_capacity(MAX_PACKET);
    payload.extend_from_slice(b"payload");
    queue.push_sequenced(INPUT, 71, payload).unwrap();
    let available = queue.data_room();
    let mut stream = InterruptedWriter {
        remaining: 14,
        bytes: Vec::new(),
    };
    queue.flush(&mut stream).unwrap();
    assert_eq!(queue.data_room(), available);
    assert!(queue.pending());
    stream.remaining = usize::MAX;
    queue.flush(&mut stream).unwrap();
    assert!(!queue.pending());
    let packet = Reader::default()
        .next(&mut io::Cursor::new(stream.bytes))
        .unwrap()
        .unwrap();
    assert_eq!(packet.kind, INPUT);
    assert_eq!(sequence(&packet.body).unwrap(), (71, b"payload".as_slice()));
}

#[test]
fn saturated_output_preserves_room_for_final_control_records() {
    let mut queue = Writer::default();
    for _ in 0..MAX_QUEUED_PACKETS - 8 {
        queue.push(OUTPUT, Vec::new()).unwrap();
    }
    assert!(matches!(
        queue.push(OUTPUT, Vec::new()),
        Err(Error::InputFull)
    ));
    queue.push(EXITED, b"exit".to_vec()).unwrap();
    let mut bytes = Vec::new();
    queue.flush(&mut bytes).unwrap();
    let mut stream = io::Cursor::new(bytes);
    let mut reader = Reader::default();
    for _ in 0..MAX_QUEUED_PACKETS - 8 {
        assert_eq!(reader.next(&mut stream).unwrap().unwrap().kind, OUTPUT);
    }
    let exit = reader.next(&mut stream).unwrap().unwrap();
    assert_eq!((exit.kind, exit.body), (EXITED, b"exit".to_vec()));
}

#[test]
fn truncated_or_oversized_record_is_not_an_orderly_channel_close() {
    let mut reader = Reader::default();
    assert!(matches!(
        reader.next(&mut io::Cursor::new([1, 0])),
        Err(Error::Protocol(_))
    ));
    let mut bytes = ((MAX_PACKET + 1) as u32).to_le_bytes().to_vec();
    bytes.push(OUTPUT);
    assert!(matches!(
        Reader::default().next(&mut io::Cursor::new(bytes)),
        Err(Error::Capacity(_))
    ));
}
