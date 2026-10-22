//! Loopback-only inspection; no caller-supplied SQL or filesystem paths.
use crate::{
    ReviewResult, now_ms,
    traces::{TraceStore, private_directory, random_id, state_directory},
};
use duckdb::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use tiny_http::{Header, Method, Response, Server};

fn query<T>(value: duckdb::Result<T>) -> ReviewResult<T> {
    value.map_err(|_| "trace_query_failed".into())
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn engine(directory: &Path) -> ReviewResult<Connection> {
    let connection = query(Connection::open_in_memory())?;
    query(connection.execute_batch(&format!("SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; SET allow_unsigned_extensions=false; SET allow_community_extensions=false; SET threads=1; SET memory_limit='128MB'; SET max_temp_directory_size='0B'; SET allowed_directories=[{}]; SET enable_external_access=false; SET lock_configuration=true;", literal(directory.to_str().ok_or("invalid_state_path")?))))?;
    Ok(connection)
}

pub fn snapshot(store: &TraceStore) -> ReviewResult<()> {
    store.prune(now_ms())?;
    let revision: i64 = store
        .connection
        .query_row(
            "SELECT revision FROM projection_state WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "trace_store_unavailable")?;
    let revision_path = store.directory.join("traces.parquet.revision");
    let destination = store.directory.join("traces.parquet");
    for path in [&revision_path, &destination] {
        if path.is_symlink() {
            return Err("unsafe_state_path".into());
        }
    }
    if destination.is_file()
        && fs::read_to_string(&revision_path).ok().as_deref() == Some(&revision.to_string())
    {
        return Ok(());
    }
    let connection = engine(&store.directory)?;
    query(connection.execute_batch("CREATE TABLE events(schema INTEGER,id VARCHAR,timestamp_ms BIGINT,outcome VARCHAR,event VARCHAR,annotation VARCHAR)"))?;
    let mut statement=store.connection.prepare("SELECT t.id,t.timestamp_ms,t.outcome,t.event,a.body FROM traces t LEFT JOIN annotations a ON a.id=t.id ORDER BY t.timestamp_ms").map_err(|_|"trace_store_unavailable")?;
    let mut rows = statement.query([]).map_err(|_| "trace_store_unavailable")?;
    let mut appender = query(connection.appender("events"))?;
    while let Some(row) = rows.next().map_err(|_| "trace_store_unavailable")? {
        let id: String = row.get(0).map_err(|_| "invalid_trace")?;
        let time: i64 = row.get(1).map_err(|_| "invalid_trace")?;
        let outcome: String = row.get(2).map_err(|_| "invalid_trace")?;
        let event: String = row.get(3).map_err(|_| "invalid_trace")?;
        let annotation: Option<String> = row.get(4).map_err(|_| "invalid_trace")?;
        query(appender.append_row(params![1, id, time, outcome, event, annotation]))?;
    }
    query(appender.flush())?;
    drop(appender);
    let partial = store.directory.join("traces.parquet.partial");
    if partial.exists() || partial.is_symlink() {
        return Err("trace_snapshot_busy".into());
    }
    // Reserve a private regular file before DuckDB writes its bounded snapshot.
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&partial).map_err(|_| "trace_snapshot_busy")?;
    let result = query(connection.execute_batch(&format!(
        "COPY events TO {} (FORMAT PARQUET, COMPRESSION ZSTD)",
        literal(partial.to_str().ok_or("invalid_state_path")?)
    )));
    if result.is_err() {
        let _ = fs::remove_file(&partial);
        return result;
    }
    fs::File::open(&partial)
        .and_then(|f| f.sync_all())
        .map_err(|_| "trace_store_unavailable")?;
    let destination = store.directory.join("traces.parquet");
    if destination.is_symlink() {
        return Err("unsafe_state_path".into());
    }
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(&destination).map_err(|_| "trace_store_unavailable")?;
    }
    fs::rename(partial, destination).map_err(|_| "trace_store_unavailable")?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut marker = options
        .open(revision_path)
        .map_err(|_| "trace_store_unavailable")?;
    marker
        .write_all(revision.to_string().as_bytes())
        .and_then(|_| marker.sync_all())
        .map_err(|_| "trace_store_unavailable")?;
    Ok(())
}

pub fn parquet_events(store: &TraceStore, before: i64, outcome: &str) -> ReviewResult<Vec<Value>> {
    parquet_page(store, before, "", outcome)
}
pub fn parquet_page(
    store: &TraceStore,
    before: i64,
    after_id: &str,
    outcome: &str,
) -> ReviewResult<Vec<Value>> {
    snapshot(store)?;
    let connection = engine(&store.directory)?;
    let mut statement=query(connection.prepare("SELECT event,annotation FROM read_parquet(?1) WHERE (timestamp_ms < ?2 OR (timestamp_ms=?2 AND id>?4)) AND (?3='' OR outcome=?3) ORDER BY timestamp_ms DESC,id LIMIT 100"))?;
    let rows = query(statement.query_map(
        params![
            store.directory.join("traces.parquet").to_str(),
            before,
            outcome,
            after_id
        ],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    ))?;
    rows.map(|row| {
        let (event, annotation) = query(row)?;
        let mut value: Value = serde_json::from_str(&event).map_err(|_| "invalid_trace")?;
        value["annotation"] = annotation
            .map(|a| serde_json::from_str(&a).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        Ok(value)
    })
    .collect()
}

pub fn command(args: &[String]) -> ReviewResult<()> {
    let store = TraceStore::open(&state_directory()?)?;
    store.prune(now_ms())?;
    let value = match args {
        [command, code, state] if command == "failure" => {
            drop(store);
            let id = crate::traces::record_failure(code, state == "cancelled")?;
            println!("{}", json!({"trace_id":id}));
            return Ok(());
        }
        [command] if command == "status" => store.status()?,
        [command] if command == "enable" => {
            store.set_consent(true)?;
            store.status()?
        }
        [command] if command == "disable" => {
            store.set_consent(false)?;
            store.status()?
        }
        [command] if command == "delete" => {
            store.delete()?;
            json!({"deleted":true,"recoverable":false})
        }
        [command, id, disposition] if command == "disposition" => {
            store.disposition(id, disposition)?;
            json!({"recorded":true})
        }
        [command, path] if command == "export" => {
            let target = Path::new(path);
            if !target.is_absolute() || target.exists() {
                return Err("invalid_export_path".into());
            }
            let parent = target.parent().ok_or("invalid_export_path")?;
            if !parent.is_dir() || parent.is_symlink() {
                return Err("invalid_export_path".into());
            }
            snapshot(&store)?;
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(target).map_err(|_| "export_failed")?;
            #[cfg(windows)]
            crate::windows::protect_directory(target)?;
            let mut input = fs::File::open(store.directory.join("traces.parquet"))
                .map_err(|_| "export_failed")?;
            std::io::copy(&mut input, &mut output).map_err(|_| "export_failed")?;
            output
                .flush()
                .and_then(|_| output.sync_all())
                .map_err(|_| "export_failed")?;
            json!({"exported":true,"sensitive":true,"retention":"user_managed"})
        }
        _ => return Err("invalid_command".into()),
    };
    println!("{value}");
    Ok(())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("static valid header")
}
pub fn serve() -> ReviewResult<()> {
    let directory = state_directory()?;
    private_directory(&directory)?;
    let server = Server::http("127.0.0.1:0").map_err(|_| "inspector_unavailable")?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or("inspector_unavailable")?;
    let host = address.to_string();
    let origin = format!("http://{host}");
    let token = random_id();
    println!(
        "{}",
        json!({"url":format!("{origin}/#{token}"),"local_only":true})
    );
    for mut request in server.incoming_requests() {
        let headers = request.headers().to_vec();
        let value_of = |name: &'static str| {
            headers
                .iter()
                .find(|h| h.field.equiv(name))
                .map(|h| h.value.as_str().to_string())
        };
        let allowed = value_of("Host").as_deref() == Some(&host)
            && value_of("Origin").is_none_or(|v| v == origin);
        let path = request.url().to_string();
        let authorized = value_of("Authorization").as_deref() == Some(&format!("Bearer {token}"));
        if allowed && authorized && request.method() == &Method::Get && path == "/api/export" {
            let result = (|| -> ReviewResult<Vec<u8>> {
                let store = TraceStore::open(&directory)?;
                snapshot(&store)?;
                fs::read(store.directory.join("traces.parquet")).map_err(|_| "export_failed".into())
            })();
            let (status, bytes) = match result {
                Ok(bytes) => (200, bytes),
                Err(code) => (400, json!({"error":code}).to_string().into_bytes()),
            };
            let response = Response::from_data(bytes)
                .with_status_code(status)
                .with_header(header("Content-Type", "application/octet-stream"))
                .with_header(header(
                    "Content-Disposition",
                    "attachment; filename=\"craig-replacements.parquet\"",
                ))
                .with_header(header("Cache-Control", "no-store"))
                .with_header(header("X-Content-Type-Options", "nosniff"));
            let _ = request.respond(response);
            continue;
        }
        let mut status = 200;
        let (mime, body) = if !allowed {
            status = 403;
            ("text/plain", "forbidden".into())
        } else if request.method() == &Method::Get && path == "/" {
            (
                "text/html",
                include_str!("../../../inspector/index.html").into(),
            )
        } else if request.method() == &Method::Get && path == "/app.js" {
            (
                "text/javascript",
                include_str!("../../../inspector/app.js").into(),
            )
        } else if request.method() == &Method::Get && path == "/style.css" {
            (
                "text/css",
                include_str!("../../../inspector/style.css").into(),
            )
        } else if !authorized {
            status = 403;
            (
                "application/json",
                json!({"error":"unauthorized"}).to_string(),
            )
        } else {
            let response = (|| -> ReviewResult<Value> {
                let store = TraceStore::open(&directory)?;
                store.prune(now_ms())?;
                if request.method() == &Method::Get && path == "/api/status" {
                    return store.status();
                }
                if request.method() != &Method::Post
                    || value_of("Origin").as_deref() != Some(&origin)
                    || value_of("Content-Type").as_deref() != Some("application/json")
                {
                    return Err("invalid_request".into());
                }
                if request.body_length().is_none_or(|n| n > 16384) {
                    return Err("invalid_request".into());
                } // 16 KiB
                let mut bytes = Vec::new();
                request
                    .as_reader()
                    .take(16385)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "invalid_request")?;
                if bytes.len() > 16384 {
                    return Err("invalid_request".into());
                }
                let input: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_request")?;
                match path.as_str() {
                    "/api/events" => Ok(json!(parquet_page(
                        &store,
                        input["before"].as_i64().unwrap_or(i64::MAX),
                        input["after_id"].as_str().unwrap_or(""),
                        input["outcome"].as_str().unwrap_or("")
                    )?)),
                    "/api/annotate" => {
                        store.annotate(
                            input["id"].as_str().ok_or("invalid_request")?,
                            input["annotation"].clone(),
                        )?;
                        Ok(json!({"saved":true}))
                    }
                    "/api/consent" => {
                        store.set_consent(input["enabled"].as_bool().ok_or("invalid_request")?)?;
                        store.status()
                    }
                    "/api/delete" if input["confirm"] == true => {
                        store.delete()?;
                        store.status()
                    }
                    _ => Err("invalid_request".into()),
                }
            })();
            match response {
                Ok(value) => ("application/json", value.to_string()),
                Err(code) => {
                    status = 400;
                    ("application/json", json!({"error":code}).to_string())
                }
            }
        };
        let response=Response::from_string(body).with_status_code(status)
            .with_header(header("Content-Type",mime)).with_header(header("Cache-Control","no-store"))
            .with_header(header("Referrer-Policy","no-referrer")).with_header(header("X-Content-Type-Options","nosniff"))
            .with_header(header("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"));
        let _ = request.respond(response);
    }
    Ok(())
}
