use std::{io::Write, net::TcpStream};

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    }
}

pub(super) fn send_http_response(stream: &mut TcpStream, status: u16, title: &str, message: &str) {
    let body = format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>{}</title>
  <style>
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: #0f172a; color: #e5e7eb; font-family: system-ui, sans-serif; }}
    main {{ max-width: 560px; padding: 40px; border: 1px solid rgba(255,255,255,.12); border-radius: 20px; background: rgba(255,255,255,.04); text-align: center; }}
    h1 {{ margin: 0 0 12px; font-size: 28px; }}
    p {{ color: #94a3b8; line-height: 1.6; }}
  </style>
</head>
<body>
  <main>
    <h1>{}</h1>
    <p>{}</p>
  </main>
</body>
</html>"#,
        html_escape(title),
        html_escape(title),
        html_escape(message)
    );
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        status_text(status),
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
