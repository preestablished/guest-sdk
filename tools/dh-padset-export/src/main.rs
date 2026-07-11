use std::{env, fs, process};

use dh_inputlog::reader::{LogReader, RecordBody};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: dh-padset-export RECORDING.dhilog");
        process::exit(2);
    });
    let bytes = fs::read(&path).unwrap_or_else(|err| {
        eprintln!("read {path}: {err}");
        process::exit(2);
    });
    let log = LogReader::parse(&bytes).unwrap_or_else(|err| {
        eprintln!("decode {path}: {err:?}");
        process::exit(1);
    });
    println!("# dh-inputlog RecordBody::PadSet frame_hint,port,buttons");
    for record in log.canonical() {
        if let RecordBody::PadSet {
            port,
            buttons,
            frame_hint,
        } = record.body()
        {
            println!("{frame_hint},{port},{buttons}");
        }
    }
}
