use std::path::PathBuf;
use std::time::{Duration, Instant};

use doxo_pdf_engine::{PdfWorker, RenderPriority, WorkerEvent};

#[test]
fn opens_and_renders_visible_pages_before_all_done() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pdf = root.join("fixtures/large-sample.pdf");
    assert!(pdf.exists(), "missing {}", pdf.display());

    let worker = PdfWorker::spawn(vec![root.join("vendor/pdfium")]);
    let handle = worker.handle();
    handle.open(pdf);

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut opened = false;
    let mut pages_ready = 0u32;
    let mut doc_id = None;
    let mut page_count = 0u32;

    while Instant::now() < deadline {
        for ev in worker.poll_events() {
            match ev {
                WorkerEvent::DocumentOpened { model, backend } => {
                    opened = true;
                    doc_id = Some(model.id);
                    page_count = model.page_count;
                    eprintln!("opened {} pages via {:?}", page_count, backend);
                    for i in 0..3.min(page_count) {
                        handle.request_page(
                            model.id,
                            doxo_document::PageId(i),
                            1.0,
                            2.0, // Retina-like DPR
                            RenderPriority::Visible,
                        );
                    }
                }
                WorkerEvent::PageReady { .. } => {
                    pages_ready += 1;
                }
                WorkerEvent::Error { message } => panic!("worker error: {message}"),
                _ => {}
            }
        }
        if opened && pages_ready >= 1 {
            assert!(page_count > pages_ready || page_count <= 3);
            assert!(doc_id.is_some());
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out; opened={opened} pages_ready={pages_ready}");
}
