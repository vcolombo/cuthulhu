// SPDX-License-Identifier: GPL-3.0-or-later
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

static ARMED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            let live =
                LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed) + layout.size() as i64;
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn write_children(zip: &mut zip::ZipWriter<std::fs::File>, count: usize) {
    const ENTRIES_PER_CHUNK: usize = 512 * 1024;
    let chunk = "0,".repeat(ENTRIES_PER_CHUNK);
    let full_chunks = (count - 1) / ENTRIES_PER_CHUNK;
    for _ in 0..full_chunks {
        zip.write_all(chunk.as_bytes()).unwrap();
    }
    let remaining = count - full_chunks * ENTRIES_PER_CHUNK;
    if remaining > 1 {
        zip.write_all("0,".repeat(remaining - 1).as_bytes())
            .unwrap();
    }
    zip.write_all(b"0").unwrap();
}

/// This is valid against the serialized schema even though its graph is nonsense. The first node
/// retains a 64 MiB `children` vector; the second then grows another from 64 to 128 MiB. The arrays
/// occupy only just over 32 MiB of JSON, so a 64 MiB manifest cap admitted a ~289 MiB parse. The
/// configured cap must reject it during the bounded read, before either vector is allocated.
#[test]
fn a_composite_dense_children_manifest_is_refused_before_deserialization() {
    const FIRST_CHILDREN: usize = 4_194_305;
    const SECOND_CHILDREN: usize = 12_582_913;
    const LOAD_ALLOCATION_BUDGET: i64 = 64 * 1024 * 1024;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dense-children.cut");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(br#"{"version":2,"document":{"nodes":{"1":{"id":1,"kind":"Layer","transform":[1.0,0.0,0.0,1.0,0.0,0.0],"style":{"stroke":255,"fill":null},"cut_line_type":"Cut","material_preset":{"state":"inherit"},"children":["#).unwrap();
    write_children(&mut zip, FIRST_CHILDREN);
    zip.write_all(br#"]},"2":{"id":2,"kind":"Layer","transform":[1.0,0.0,0.0,1.0,0.0,0.0],"style":{"stroke":255,"fill":null},"cut_line_type":"Cut","material_preset":{"state":"inherit"},"children":["#).unwrap();
    write_children(&mut zip, SECOND_CHILDREN);
    zip.write_all(br#"]}},"root":1,"ids":2,"artboard":{"x":0.0,"y":0.0,"w":330.0,"h":3000.0},"machine":null}}"#).unwrap();
    zip.finish().unwrap();

    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    let result = fileio::load_project(&path);
    ARMED.store(false, Ordering::Relaxed);

    match &result {
        Err(fileio::IoError::Io(message)) if message == "manifest.json is larger than 32 MiB" => {}
        Err(error) => panic!("the unsafe composite failed for the wrong reason: {error}"),
        Ok(_) => panic!(
            "the unsafe composite loaded after allocating {} MiB",
            PEAK.load(Ordering::Relaxed) / (1024 * 1024)
        ),
    }
    assert!(
        PEAK.load(Ordering::Relaxed) <= LOAD_ALLOCATION_BUDGET,
        "refusal allocated {} MiB before returning",
        PEAK.load(Ordering::Relaxed) / (1024 * 1024)
    );
}
