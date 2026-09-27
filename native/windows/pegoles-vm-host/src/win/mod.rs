//! Windows side of the helper: the broker client, the guest socket and
//! the serial pipe, behind the platform-free [`Machine`] trait.

mod broker;
mod link;
mod serial;
mod util;

use std::collections::HashMap;
use std::sync::Arc;

use pegoles_broker_proto::{CreateSpec, Request};
use pegoles_computer::vmhost_proto::CreateParams;

use crate::dispatch::Machine;
use crate::Out;

struct Running {
    link: link::GuestLink,
    serial: Option<serial::SerialPump>,
}

pub struct WinMachine {
    out: Arc<Out>,
    broker: Option<broker::Broker>,
    running: HashMap<String, Running>,
}

impl WinMachine {
    pub fn new(out: Arc<Out>) -> Self {
        Self {
            out,
            broker: None,
            running: HashMap::new(),
        }
    }

    /// The broker connection, (re)opened on demand; one failure drops it
    /// so the next call starts clean.
    fn call(&mut self, request: &Request) -> Result<pegoles_broker_proto::Response, String> {
        if self.broker.is_none() {
            self.broker = Some(broker::Broker::connect()?);
        }
        let result = self
            .broker
            .as_mut()
            .map(|b| b.call(request))
            .unwrap_or_else(|| Err("broker unavailable".into()));
        match result {
            Ok(response) if response.ok => Ok(response),
            Ok(response) => Err(response
                .error
                .map(|e| format!("{}: {}", e.code, e.message))
                .unwrap_or_else(|| "the broker refused".into())),
            Err(e) => {
                self.broker = None;
                Err(e)
            }
        }
    }

    fn stop_links(&mut self, id: &str) {
        if let Some(mut running) = self.running.remove(id) {
            running.link.close();
            if let Some(serial) = running.serial.as_mut() {
                serial.close();
            }
        }
    }
}

impl Machine for WinMachine {
    fn boot(&mut self, params: &CreateParams) -> Result<(), String> {
        let id = params.computer_id.clone();
        self.stop_links(&id);
        // COM1 pipe first: HCS connects to it when the VM is created.
        let serial = (!params.serial_log_path.is_empty())
            .then(|| serial::SerialPump::open(&id, &params.serial_log_path))
            .transpose()
            .unwrap_or(None);
        let created = self.call(&Request::Create(CreateSpec {
            computer_id: id.clone(),
            disk: params.disk_path.clone(),
            vcpus: params.vcpus,
            memory_mb: params.memory_mb - params.memory_mb % 2,
        }));
        let runtime_id = match created.and_then(|r| {
            r.runtime_id
                .ok_or_else(|| "the broker returned no VM id".into())
        }) {
            Ok(runtime_id) => runtime_id,
            Err(e) => {
                if let Some(mut s) = serial {
                    s.close();
                }
                return Err(e);
            }
        };
        // Start connecting before the VM runs: the link retries until the
        // runtime listens.
        let link = link::GuestLink::open(&id, &runtime_id, self.out.clone())?;
        if let Err(e) = self.call(&Request::Start {
            computer_id: id.clone(),
        }) {
            let mut link = link;
            link.close();
            let _ = self.call(&Request::Terminate {
                computer_id: id.clone(),
            });
            if let Some(mut s) = serial {
                s.close();
            }
            return Err(e);
        }
        self.running.insert(id, Running { link, serial });
        Ok(())
    }

    fn pause(&mut self, id: &str) -> Result<(), String> {
        self.call(&Request::Pause {
            computer_id: id.into(),
        })
        .map(|_| ())
    }

    fn resume(&mut self, id: &str) -> Result<(), String> {
        self.call(&Request::Resume {
            computer_id: id.into(),
        })
        .map(|_| ())
    }

    fn halt(&mut self, id: &str) -> Result<(), String> {
        self.stop_links(id);
        match self.call(&Request::Shutdown {
            computer_id: id.into(),
        }) {
            Ok(_) => Ok(()),
            // The broker already forgot it (it ended with its lease).
            Err(e) if e.starts_with("unknown_computer") => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn guest_send(&mut self, id: &str, payload: &str) -> Result<(), String> {
        match self.running.get(id) {
            Some(running) => running.link.send(payload),
            None => Err("the computer is not running".into()),
        }
    }

    fn guest_connected(&self, id: &str) -> bool {
        self.running.get(id).is_some_and(|r| r.link.connected())
    }

    fn guest_kick(&mut self, id: &str) {
        if let Some(running) = self.running.get(id) {
            running.link.kick("kicked");
        }
    }

    fn shutdown(&mut self) {
        let ids: Vec<String> = self.running.keys().cloned().collect();
        for id in ids {
            let _ = self.halt(&id);
        }
        // Dropping the broker connection ends the lease for anything left.
        self.broker = None;
    }
}
