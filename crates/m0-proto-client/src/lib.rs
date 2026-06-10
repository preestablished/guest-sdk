#![forbid(unsafe_code)]

pub fn sample_page_ref() -> determinism_proto::common::v1::PageRef {
    determinism_proto::common::v1::PageRef {
        snapshot_ref: vec![0; 32],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_proto_client_compiles() {
        assert_eq!(super::sample_page_ref().snapshot_ref.len(), 32);
    }
}
