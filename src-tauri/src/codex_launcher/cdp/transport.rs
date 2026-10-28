use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};
use url::Url;

pub(super) struct CdpConnection {
    reader: BufReader<TcpStream>,
    next_id: i64,
    events: VecDeque<Value>,
}

impl CdpConnection {
    pub(super) fn connect(websocket_url: &str) -> Result<Self, String> {
        let url =
            Url::parse(websocket_url).map_err(|err| format!("CDP WebSocket URL 无效: {err}"))?;
        if url.scheme() != "ws" {
            return Err("CDP WebSocket 仅支持 ws://".to_string());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "CDP WebSocket URL 缺少 host".to_string())?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| "CDP WebSocket URL 缺少端口".to_string())?;
        let mut path = url.path().to_string();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        let stream = TcpStream::connect((host, port))
            .map_err(|err| format!("连接 CDP WebSocket 失败: {err}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|err| format!("设置 CDP WebSocket read timeout 失败: {err}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|err| format!("设置 CDP WebSocket write timeout 失败: {err}"))?;
        let mut reader = BufReader::new(stream);
        let key = BASE64_STANDARD.encode([0u8; 16]);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        reader
            .get_mut()
            .write_all(request.as_bytes())
            .map_err(|err| format!("发送 CDP WebSocket 握手失败: {err}"))?;
        let mut response = String::new();
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .map_err(|err| format!("读取 CDP WebSocket 握手失败: {err}"))?
                == 0
            {
                return Err("CDP WebSocket 握手提前关闭".to_string());
            }
            response.push_str(&line);
            if response.len() > 8192 {
                return Err("CDP WebSocket 握手响应过大".to_string());
            }
            if line == "\r\n" {
                break;
            }
        }
        if !response.starts_with("HTTP/1.1 101") && !response.starts_with("HTTP/1.0 101") {
            return Err(format!("CDP WebSocket 握手未升级协议: {}", response.trim()));
        }
        Ok(Self {
            reader,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    pub(super) fn command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        send_ws_frame(
            self.reader.get_mut(),
            0x1,
            json!({ "id": id, "method": method, "params": params })
                .to_string()
                .as_bytes(),
        )?;
        loop {
            let text = read_ws_text(&mut self.reader)?;
            let value: Value =
                serde_json::from_str(&text).map_err(|err| format!("解析 CDP 响应失败: {err}"))?;
            if value.get("id").and_then(Value::as_i64) != Some(id) {
                if value.get("method").is_some() {
                    self.events.push_back(value);
                }
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(format!("CDP {method} 失败: {error}"));
            }
            return Ok(value);
        }
    }

    pub(super) fn take_events(&mut self) -> Vec<Value> {
        self.events.drain(..).collect()
    }
}

fn send_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    let mask = [0x13u8, 0x37, 0x5a, 0xc0];
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream
        .write_all(&frame)
        .map_err(|err| format!("发送 CDP WebSocket frame 失败: {err}"))
}

fn read_ws_text(reader: &mut BufReader<TcpStream>) -> Result<String, String> {
    let mut message = Vec::new();
    let mut continuing = false;
    loop {
        let mut header = [0u8; 2];
        reader
            .read_exact(&mut header)
            .map_err(|err| format!("读取 CDP WebSocket frame 失败: {err}"))?;
        let opcode = header[0] & 0x0f;
        let final_frame = header[0] & 0x80 != 0;
        let masked = header[1] & 0x80 != 0;
        let mut len = u64::from(header[1] & 0x7f);
        if len == 126 {
            let mut buffer = [0u8; 2];
            reader
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取 CDP WebSocket frame 长度失败: {err}"))?;
            len = u64::from(u16::from_be_bytes(buffer));
        } else if len == 127 {
            let mut buffer = [0u8; 8];
            reader
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取 CDP WebSocket frame 长度失败: {err}"))?;
            len = u64::from_be_bytes(buffer);
        }
        if len > 16 * 1024 * 1024 || message.len() as u64 + len > 16 * 1024 * 1024 {
            return Err("CDP WebSocket message 过大".to_string());
        }
        let mut mask = [0u8; 4];
        if masked {
            reader
                .read_exact(&mut mask)
                .map_err(|err| format!("读取 CDP WebSocket mask 失败: {err}"))?;
        }
        let mut payload = vec![0u8; len as usize];
        reader
            .read_exact(&mut payload)
            .map_err(|err| format!("读取 CDP WebSocket payload 失败: {err}"))?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        match opcode {
            0x1 if !continuing => {
                message.extend_from_slice(&payload);
                continuing = !final_frame;
            }
            0x0 if continuing => {
                message.extend_from_slice(&payload);
                continuing = !final_frame;
            }
            0x8 => return Err("CDP WebSocket 已关闭".to_string()),
            0x9 => {
                send_ws_frame(reader.get_mut(), 0xA, &payload)?;
                continue;
            }
            0xA => continue,
            _ => return Err(format!("CDP WebSocket frame 顺序无效: opcode={opcode}")),
        }
        if !continuing {
            return String::from_utf8(message)
                .map_err(|err| format!("CDP WebSocket 文本不是 UTF-8: {err}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    fn server_frame(final_frame: bool, opcode: u8, text: &[u8]) -> Vec<u8> {
        assert!(text.len() < 126);
        let mut frame = vec![
            if final_frame { 0x80 } else { 0 } | opcode,
            text.len() as u8,
        ];
        frame.extend_from_slice(text);
        frame
    }

    fn accept_client(listener: TcpListener, first_event: &[u8]) -> BufReader<TcpStream> {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        // Coalesce the first WebSocket event with the HTTP upgrade response.
        let mut reply = b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n".to_vec();
        reply.extend_from_slice(first_event);
        reader.get_mut().write_all(&reply).unwrap();
        let command = read_ws_text(&mut reader).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&command).unwrap()["id"], 1);
        reader
    }

    #[test]
    fn websocket_preserves_coalesced_events_and_fragmented_utf8_with_ping() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let event = server_frame(
                true,
                1,
                br#"{"method":"Debugger.scriptParsed","params":{}}"#,
            );
            let mut reader = accept_client(listener, &event);
            let response = "{\"id\":1,\"result\":{\"value\":\"分片响应\"}}".as_bytes();
            let split = response.iter().position(|byte| *byte >= 0x80).unwrap() + 1;
            let mut frames = server_frame(false, 1, &response[..split]);
            frames.extend(server_frame(true, 9, b"p"));
            frames.extend(server_frame(true, 0, &response[split..]));
            reader.get_mut().write_all(&frames).unwrap();
            let mut pong = [0u8; 7];
            reader.read_exact(&mut pong).unwrap();
            assert_eq!(pong[0], 0x8a);
        });
        let mut connection =
            CdpConnection::connect(&format!("ws://127.0.0.1:{port}/fixture")).unwrap();
        let response = connection.command("Runtime.evaluate", json!({})).unwrap();
        assert_eq!(response["result"]["value"], "分片响应");
        assert_eq!(
            connection.take_events()[0]["method"],
            "Debugger.scriptParsed"
        );
        worker.join().unwrap();
    }

    #[test]
    fn websocket_returns_the_original_cdp_error_and_method() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let mut reader = accept_client(listener, &[]);
            reader
                .get_mut()
                .write_all(&server_frame(
                    true,
                    1,
                    br#"{"id":1,"error":{"code":-32000,"message":"fixture rejected"}}"#,
                ))
                .unwrap();
        });
        let mut connection =
            CdpConnection::connect(&format!("ws://127.0.0.1:{port}/fixture")).unwrap();
        let error = connection
            .command("Debugger.setBreakpointByUrl", json!({}))
            .unwrap_err();
        assert!(error.contains("Debugger.setBreakpointByUrl"), "{error}");
        assert!(error.contains("fixture rejected"), "{error}");
        assert!(error.contains("-32000"), "{error}");
        worker.join().unwrap();
    }
}
