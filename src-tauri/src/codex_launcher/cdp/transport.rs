use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use url::Url;

pub(super) struct CdpConnection {
    reader: BufReader<TcpStream>,
    next_id: i64,
    events: VecDeque<Value>,
    protocol_error: Option<Value>,
}

impl CdpConnection {
    pub(super) fn connect(websocket_url: &str) -> Result<Self, String> {
        Self::connect_until(websocket_url, Instant::now() + Duration::from_secs(5))
    }

    pub(super) fn connect_until(websocket_url: &str, deadline: Instant) -> Result<Self, String> {
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
        let address = (host, port)
            .to_socket_addrs()
            .map_err(|err| format!("解析 CDP WebSocket 地址失败: {err}"))?
            .next()
            .ok_or_else(|| "CDP WebSocket 地址缺失".to_string())?;
        let stream = TcpStream::connect_timeout(&address, remaining(deadline)?)
            .map_err(|err| format!("连接 CDP WebSocket 失败: {err}"))?;
        let mut reader = BufReader::new(stream);
        let key = BASE64_STANDARD.encode([0u8; 16]);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        write_all_until(reader.get_mut(), request.as_bytes(), deadline)?;
        let mut response = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            read_exact_until(&mut reader, &mut byte, deadline)?;
            response.push(byte[0]);
            if response.len() > 8192 {
                return Err("CDP WebSocket 握手响应过大".to_string());
            }
            if response.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let response = String::from_utf8_lossy(&response);
        if !response.starts_with("HTTP/1.1 101") && !response.starts_with("HTTP/1.0 101") {
            return Err(format!("CDP WebSocket 握手未升级协议: {}", response.trim()));
        }
        Ok(Self {
            reader,
            next_id: 1,
            events: VecDeque::new(),
            protocol_error: None,
        })
    }

    pub(super) fn command(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.command_until(method, params, Instant::now() + Duration::from_secs(5))
    }

    pub(super) fn command_until(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.protocol_error = None;
        self.next_id += 1;
        let started = Instant::now();
        send_ws_frame_until(
            self.reader.get_mut(),
            0x1,
            json!({ "id": id, "method": method, "params": params })
                .to_string()
                .as_bytes(),
            deadline,
        )?;
        loop {
            let text =
                read_ws_text_until(&mut self.reader, deadline).map_err(|error| {
                    format!(
                    "CDP {method} id={id} elapsedMs={} queuedEvents={} lastEvent={} 失败: {error}",
                    started.elapsed().as_millis(), self.events.len(),
                    self.events.back().and_then(|event| event["method"].as_str()).unwrap_or("none")
                )
                })?;
            let value: Value =
                serde_json::from_str(&text).map_err(|err| format!("解析 CDP 响应失败: {err}"))?;
            if value.get("id").and_then(Value::as_i64) != Some(id) {
                if value.get("method").is_some() {
                    self.events.push_back(value);
                }
                continue;
            }
            if let Some(error) = value.get("error") {
                self.protocol_error = Some(error.clone());
                return Err(format!("CDP {method} 失败: {error}"));
            }
            return Ok(value);
        }
    }

    pub(super) fn take_events(&mut self) -> Vec<Value> {
        self.events.drain(..).collect()
    }

    pub(super) fn take_protocol_error(&mut self) -> Option<Value> {
        self.protocol_error.take()
    }

    pub(super) fn interrupt_handle(&self) -> Result<TcpStream, String> {
        self.reader
            .get_ref()
            .try_clone()
            .map_err(|err| format!("复制 CDP cancellation socket 失败: {err}"))
    }

    pub(super) fn receive_event_until(&mut self, deadline: Instant) -> Result<Value, String> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        let text = read_ws_text_until(&mut self.reader, deadline)?;
        let event: Value =
            serde_json::from_str(&text).map_err(|error| format!("解析 CDP event 失败: {error}"))?;
        if event.get("method").is_none() {
            return Err(format!("等待 CDP event 时收到未请求的响应: {event}"));
        }
        Ok(event)
    }

    pub(super) fn poll_event_until(
        &mut self,
        wait_deadline: Instant,
        message_deadline: Instant,
    ) -> Result<Option<Value>, String> {
        if !self.events.is_empty() || !self.reader.buffer().is_empty() {
            return self.receive_event_until(message_deadline).map(Some);
        }
        self.reader
            .get_mut()
            .set_read_timeout(Some(remaining(wait_deadline)?))
            .map_err(|error| format!("设置 CDP event wait deadline 失败: {error}"))?;
        let mut byte = [0u8; 1];
        match self.reader.get_ref().peek(&mut byte) {
            Ok(0) => Err("CDP WebSocket 已关闭".to_string()),
            Ok(_) => self.receive_event_until(message_deadline).map(Some),
            // peek has consumed no frame bytes. This is an idle event wait, not
            // a partially read frame or a failed RPC that can be retried.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(format!("等待 CDP event 失败: {error}")),
        }
    }

    #[cfg(test)]
    pub(super) fn queue_fixture_events(&mut self, events: impl IntoIterator<Item = Value>) {
        self.events.extend(events);
    }
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| "CDP operation deadline exceeded".to_string())
}

fn read_exact_until(
    reader: &mut BufReader<TcpStream>,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < buffer.len() {
        reader
            .get_mut()
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|err| format!("设置 CDP read deadline 失败: {err}"))?;
        let count = reader.read(&mut buffer[offset..]).map_err(|err| {
            let deadline_state = if Instant::now() >= deadline {
                " operation deadline exceeded"
            } else {
                ""
            };
            format!("读取 CDP WebSocket{deadline_state} 失败: {err}")
        })?;
        if count == 0 {
            return Err("CDP WebSocket 已关闭".to_string());
        }
        offset += count;
    }
    Ok(())
}

fn write_all_until(stream: &mut TcpStream, buffer: &[u8], deadline: Instant) -> Result<(), String> {
    let mut offset = 0;
    while offset < buffer.len() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|err| format!("设置 CDP write deadline 失败: {err}"))?;
        let count = stream
            .write(&buffer[offset..])
            .map_err(|err| format!("发送 CDP WebSocket 失败: {err}"))?;
        if count == 0 {
            return Err("CDP WebSocket write 返回0".to_string());
        }
        offset += count;
    }
    Ok(())
}

fn send_ws_frame_until(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
    deadline: Instant,
) -> Result<(), String> {
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
    write_all_until(stream, &frame, deadline)
}

fn read_ws_text_until(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<String, String> {
    let mut message = Vec::new();
    let mut continuing = false;
    loop {
        let mut header = [0u8; 2];
        read_exact_until(reader, &mut header, deadline)?;
        let opcode = header[0] & 0x0f;
        let final_frame = header[0] & 0x80 != 0;
        let masked = header[1] & 0x80 != 0;
        let mut len = u64::from(header[1] & 0x7f);
        if len == 126 {
            let mut buffer = [0u8; 2];
            read_exact_until(reader, &mut buffer, deadline)?;
            len = u64::from(u16::from_be_bytes(buffer));
        } else if len == 127 {
            let mut buffer = [0u8; 8];
            read_exact_until(reader, &mut buffer, deadline)?;
            len = u64::from_be_bytes(buffer);
        }
        if len > 16 * 1024 * 1024 || message.len() as u64 + len > 16 * 1024 * 1024 {
            return Err("CDP WebSocket message 过大".to_string());
        }
        let mut mask = [0u8; 4];
        if masked {
            read_exact_until(reader, &mut mask, deadline)?;
        }
        let mut payload = vec![0u8; len as usize];
        read_exact_until(reader, &mut payload, deadline)?;
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
                send_ws_frame_until(reader.get_mut(), 0xA, &payload, deadline)?;
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
pub(super) fn read_fixture_text(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<String, String> {
    read_ws_text_until(reader, deadline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::BufRead, net::TcpListener, thread};

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
        let command =
            read_ws_text_until(&mut reader, Instant::now() + Duration::from_secs(5)).unwrap();
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

    #[test]
    fn rpc_deadline_is_not_extended_by_unrelated_events() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = thread::spawn(move || {
            let mut reader = accept_client(listener, &[]);
            let frame = server_frame(
                true,
                1,
                br#"{"method":"Debugger.scriptParsed","params":{}}"#,
            );
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if reader.get_mut().write_all(&frame).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let mut connection =
            CdpConnection::connect(&format!("ws://127.0.0.1:{port}/fixture")).unwrap();
        let started = Instant::now();
        let error = connection
            .command_until(
                "Debugger.getScriptSource",
                json!({}),
                started + Duration::from_millis(120),
            )
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(600), "{error}");
        assert!(error.contains("Debugger.getScriptSource id=1"), "{error}");
        assert!(error.contains("queuedEvents="), "{error}");
        assert!(error.contains("deadline exceeded"), "{error}");
        drop(connection);
        worker.join().unwrap();
    }
}
