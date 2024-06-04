use std::{
    env, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use actix_files::NamedFile;
use actix_web::{get, post, web::Data, App, HttpServer, Responder};

use actix_web_lab::{
    respond::Html,
    sse::{self},
};
use actix_web_sse::Broadcaster;
use log::{error, info, warn};

use notify::{event::ModifyKind, EventKind, RecommendedWatcher, Watcher};
use tokio::{
    runtime::Handle,
    sync::mpsc::{self, Sender},
};

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

#[post("/stop")]
async fn stop(stop_tx: Data<Sender<()>>) -> impl Responder {
    stop_tx.send(()).await.unwrap();
    "Server stopped"
}

async fn create_watcher(broadcaster: Data<Arc<Broadcaster>>, path: &Path) -> RecommendedWatcher {
    let handle = Handle::current();
    let file_name = path.file_name().unwrap().to_owned();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else {
            return;
        };
        let EventKind::Modify(ModifyKind::Data(_)) = event.kind else {
            return;
        };

        if event
            .paths
            .iter()
            .filter_map(|r| r.file_name())
            .any(|p| p == file_name)
        {
            info!("file modified");
            handle.block_on(broadcaster.broadcast("update"));
        }
    })
    .unwrap();
    watcher
        .watch(path.parent().unwrap(), notify::RecursiveMode::NonRecursive)
        .unwrap();

    watcher
}

const DEFAULT_PORT: u16 = 8999;

#[actix_web::main]
async fn main() -> io::Result<()> {
    let mut args = env::args();
    if args.len() < 2 {
        println!("Usage: {} <pdf_path> [port]", args.next().unwrap());
    }
    args.next();
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    let state = Data::new(AppState {
        pdf_path: args.next().unwrap().into(),
    });
    let port = if let Some(port) = args.next() {
        port.parse().unwrap_or_else(|_| {
            error!("invalid port, using default port");
            DEFAULT_PORT
        })
    } else {
        DEFAULT_PORT
    };

    if state.pdf_path.extension().unwrap() != "pdf" {
        warn!("PDF file should have a .pdf extension, are you sure this is a PDF file?");
    }
    let broadcaster = Data::new(Broadcaster::create());
    let mut watcher = create_watcher(broadcaster.clone(), &state.pdf_path).await;

    let state_copy = state.clone();
    let (stop_tx, mut stop_rx) = mpsc::channel(1);
    let server = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(stop_tx.clone()))
            .app_data(state_copy.clone())
            .app_data(broadcaster.clone())
            .service(get_pdf)
            .service(sse_listen)
            .service(index)
            .service(stop)
    })
    .workers(2)
    .bind(("127.0.0.1", port))?
    .run();

    let handle = server.handle();
    tokio::spawn(async move {
        if let Some(()) = stop_rx.recv().await {
            info!("Stopping server");
            handle.stop(false).await;
        }
    });

    info!("Server running on http://127.0.0.1:{}", port);
    let res = server.await;
    watcher.unwatch(state.pdf_path.parent().unwrap()).unwrap();
    res
}
