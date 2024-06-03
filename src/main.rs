use std::{env, io, path::PathBuf, sync::Arc};

use actix_files::NamedFile;
use actix_web::{get, web::Data, App, HttpServer, Responder};
use actix_web_lab::{respond::Html, sse};
use actix_web_sse::Broadcaster;
use log::{info, warn};

use notify::{RecommendedWatcher, Watcher};
use tokio::runtime::Handle;

struct AppState {
    pdf_path: PathBuf,
}

#[get("/pdf")]
async fn get_pdf(state: Data<AppState>) -> actix_web::Result<NamedFile> {
    Ok(NamedFile::open(state.pdf_path.clone())?)
}

#[get("/listen")]
async fn sse_listen(_: Data<AppState>, broadcaster: Data<Arc<Broadcaster>>) -> impl Responder {
    broadcaster.new_client(sse::Data::new("connected")).await
}

#[get("/")]
async fn index() -> impl Responder {
    Html::new(include_str!("index.html"))
}

async fn create_watcher(broadcaster: Data<Arc<Broadcaster>>) -> RecommendedWatcher {
    let handle = Handle::current();
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event.kind.is_modify() {
                info!("file modified");
                handle.block_on(broadcaster.broadcast("update"));
            }
        };
    })
    .unwrap();

    watcher
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    let mut args = env::args();
    if args.len() < 2 {
        println!("Usage: {} <pdf_path>", args.next().unwrap());
    }
    args.next();
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    let state = Data::new(AppState {
        pdf_path: args.next().unwrap().into(),
    });
    if !state.pdf_path.ends_with(".pdf") {
        warn!("PDF file should have a .pdf extension, are you sure this is a PDF file?");
    }
    let broadcaster = Data::new(Broadcaster::create());
    let mut watcher = create_watcher(broadcaster.clone()).await;

    watcher
        .watch(&state.pdf_path, notify::RecursiveMode::NonRecursive)
        .unwrap();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(broadcaster.clone())
            .service(get_pdf)
            .service(sse_listen)
            .service(index)
    })
    .workers(2)
    .bind(("127.0.0.1", 8080))?
    .run();

    info!("Server running on http://127.0.0.1:8080");
    server.await
}
