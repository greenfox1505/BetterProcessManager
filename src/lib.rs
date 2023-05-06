use std::io::{Read, Write};
use std::process::Stdio;
use std::process::{Child, Command};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};

use godot::engine::{Engine, RefCounted};
use godot::prelude::*;

struct ProcessPlugin;
#[gdextension]
unsafe impl ExtensionLibrary for ProcessPlugin {}

#[derive(GodotClass)]
#[class(base=Node)]
struct ProcessManager {
    #[export]
    pub start_on_ready: bool,
    #[export]
    pub cmd: GodotString,
    #[export]
    pub args: PackedStringArray,

    raw_process: Option<RawProcess>,
    #[base]
    base: Base<Node>,
}

#[godot_api]
impl NodeVirtual for ProcessManager {
    fn init(base: Base<Node>) -> Self {
        Self {
            start_on_ready: false,
            cmd: GodotString::from(""),
            args: PackedStringArray::new(),
            raw_process: None,
            base,
        }
    }
    fn ready(&mut self) {
        if Engine::singleton().is_editor_hint() {
            return;
        }
        if self.start_on_ready {
            self.start();
        }
    }
    fn process(&mut self, delta: f64) {
        if let Some(rp) = self.raw_process.take() {
            let out = rp.read_stdout();
            if out.len() != 0 {
                self.base
                    .emit_signal("stdout".into(), &[GodotString::from(out).to_variant()]);
            }

            let err = rp.read_stderr();
            if err.len() != 0 {
                self.base
                    .emit_signal("stderr".into(), &[GodotString::from(err).to_variant()]);
            }

            self.raw_process = Some(rp);
        }
    }
}

#[godot_api]
impl ProcessManager {
    #[func]
    fn start(&mut self) {
        //start cmd
        let cmd = self.cmd.to_string();
        let args: Vec<String> = self
            .args
            .to_vec()
            .iter()
            .map(|i: &GodotString| i.to_string())
            .collect();
        let rp = RawProcess::new(cmd, args, true);
        self.raw_process = Some(rp);
    }

    #[func]
    fn write(&mut self, s: GodotString) {
        match self.raw_process.take() {
            Some(rp) => {
                rp.write(s.to_string().as_bytes());
                self.raw_process = Some(rp);
            }
            _ => {
                godot_error!("Can't write to closed process!");
            }
        }
    }

    // #[func]
    // fn qwer(&mut self){
    // }

    #[signal]
    fn stdout();
    #[signal]
    fn stderr();
}

#[derive(GodotClass)]
#[class(base=RefCounted)]
struct Process {
    #[base]
    base: Base<RefCounted>,
}

#[godot_api]
impl NodeVirtual for Process {
    fn init(base: Base<RefCounted>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl Process {}

struct RawProcess {
    stdin_tx: Sender<u8>,
    stdout_rx: Receiver<u8>,
    stderr_rx: Receiver<u8>,
    handle_stdout: JoinHandle<Result<(), String>>,
    handle_stdin: JoinHandle<Result<(), String>>,
    child: Child,
}

impl RawProcess {
    fn new(cmd: String, args: Vec<String>, bundle_stderr: bool) -> Self {
        let (stdout_tx, stdout_rx) = channel();
        let (stderr_tx, stderr_rx) = channel();
        let (stdin_tx, stdin_rx) = channel();
        let mut child = Command::new(cmd)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        let handle_stdout = {
            let stdout_tx = stdout_tx.clone();
            let stdout = child.stdout.take();
            thread::spawn(move || match stdout {
                Some(stdout) => {
                    for i in stdout.bytes() {
                        let _ = stdout_tx.send(i.unwrap());
                    }
                    Ok(())
                }
                None => Err("StdOut didn't init correctly".into()),
            })
        };

        let handle_stdin = {
            let stdin = child.stdin.take();
            thread::spawn(move || {
                //this maybe needs to migrate to the above scope so we can process the handles correctly
                let mut a = stdin.unwrap();
                stdin_rx.iter().for_each(|v| {
                    let _ = a.write(&[v]);
                    a.flush();
                });
                Ok(())
            })
        };

        let stderr = child.stderr.take();
        let stderr_tx = stderr_tx.clone();
        let _ = thread::spawn(move || match stderr {
            Some(stderr) => {
                for i in stderr.bytes() {
                    let _ = stderr_tx.send(i.unwrap());
                }
            }
            None => {}
        });

        Self {
            stdout_rx,
            stderr_rx,
            stdin_tx,
            handle_stdout,
            handle_stdin,
            child,
        }
    }
    fn write(&self, text: &[u8]) {
        //should this be a str? u8 array?
        text.iter().for_each(|i| {
            let _ = self.stdin_tx.send(*i);
        })
    }
    fn read_stderr(&self) -> String {
        let a: Vec<u8> = self.stderr_rx.try_iter().collect();
        String::from_utf8(a).unwrap()
    }
    fn read_stdout(&self) -> String {
        let a: Vec<u8> = self.stdout_rx.try_iter().collect();
        String::from_utf8(a).unwrap()
    }

    fn read(&self) -> String {
        format!("{}{}", self.read_stdout(), self.read_stderr())
    }
    fn is_finished(&self) -> bool {
        self.handle_stdout.is_finished()
    }
}

impl Drop for RawProcess {
    fn drop(&mut self) {
        match self.child.kill() {
            Ok(_) => {}
            Err(_) => godot_error!("Failed to kill child process!"),
        }
    }
}
