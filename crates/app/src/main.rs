use std::path::PathBuf;

use anyhow::Result;
use doxo_pdf_engine::PdfWorker;
use doxo_platform_macos::pdfium_search_paths;
use doxo_ui::DoxoApp;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "doxo=info,doxo_pdf_engine=info".into()),
        )
        .init();

    // Touch scaffolding crates so the workspace layout stays intentional.
    let _ = doxo_text_engine::shape_text;
    let _ = doxo_graphics::begin_stroke;
    let _ = doxo_pdf_lowlevel::incremental_save_stub;
    doxo_platform_macos::register_document_types_stub();

    let initial = std::env::args().nth(1).map(PathBuf::from);
    if let Some(ref path) = initial {
        info!(path = %path.display(), "opening from CLI");
    }

    let search = pdfium_search_paths();
    info!(?search, "pdfium search paths");
    let worker = PdfWorker::spawn(search);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("doxo"),
        ..Default::default()
    };

    eframe::run_native(
        "doxo",
        native_options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(DoxoApp::new(cc, worker, initial)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
