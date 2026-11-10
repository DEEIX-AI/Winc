//! LSP 注册表：管理多个语言服务器实例。

use crate::client::{Client, ClientId};
use crate::{Error, Result};
use parking_lot::RwLock;
use slotmap::SlotMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// LSP 服务器注册表。
pub struct Registry {
    /// 所有客户端实例（按 ID 索引）
    clients: RwLock<SlotMap<ClientId, Arc<Client>>>,
    /// 按名称索引的客户端（支持多实例）
    clients_by_name: RwLock<HashMap<String, Vec<Arc<Client>>>>,
}

impl Registry {
    /// 创建新的注册表。
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(SlotMap::with_key()),
            clients_by_name: RwLock::new(HashMap::new()),
        }
    }

    /// 启动一个新的语言服务器。
    pub fn start_server(
        &self,
        name: String,
        command: &str,
        args: &[String],
        root_path: PathBuf,
    ) -> Result<Arc<Client>> {
        // 先预留一个 slot 来获取 ID
        let mut clients = self.clients.write();
        let id = clients.insert_with_key(|id| {
            // 这个闭包会得到即将分配的 ID
            Arc::new(
                Client::spawn(id, name.clone(), command, args, root_path.clone())
                    .expect("Failed to spawn client"),
            )
        });

        let client = clients[id].clone();
        drop(clients); // 释放锁

        // 添加到名称索引
        self.clients_by_name
            .write()
            .entry(name)
            .or_insert_with(Vec::new)
            .push(Arc::clone(&client));

        Ok(client)
    }

    /// 通过 ID 获取客户端。
    pub fn get(&self, id: ClientId) -> Option<Arc<Client>> {
        self.clients.read().get(id).map(Arc::clone)
    }

    /// 通过名称获取客户端（返回第一个实例）。
    pub fn get_by_name(&self, name: &str) -> Option<Arc<Client>> {
        self.clients_by_name
            .read()
            .get(name)
            .and_then(|clients| clients.first().map(Arc::clone))
    }

    /// 通过名称获取所有客户端实例。
    pub fn get_all_by_name(&self, name: &str) -> Vec<Arc<Client>> {
        self.clients_by_name
            .read()
            .get(name)
            .map(|clients| clients.clone())
            .unwrap_or_default()
    }

    /// 停止服务器。
    pub fn stop(&self, id: ClientId) -> Result<()> {
        let client = self
            .clients
            .write()
            .remove(id)
            .ok_or_else(|| Error::ServerNotFound(format!("{:?}", id)))?;

        // 从名称索引中移除
        if let Some(clients) = self.clients_by_name.write().get_mut(client.name()) {
            clients.retain(|c| c.id() != id);
        }

        client.shutdown()?;
        Ok(())
    }

    /// 停止指定名称的所有服务器。
    pub fn stop_by_name(&self, name: &str) -> Result<()> {
        let clients = self
            .clients_by_name
            .write()
            .remove(name)
            .unwrap_or_default();

        for client in clients {
            self.clients.write().remove(client.id());
            client.shutdown()?;
        }

        Ok(())
    }

    /// 获取所有客户端。
    pub fn iter_clients(&self) -> Vec<Arc<Client>> {
        self.clients.read().values().map(Arc::clone).collect()
    }

    /// 关闭所有服务器。
    pub fn shutdown_all(&self) {
        let clients: Vec<_> = self.clients.read().values().cloned().collect();

        for client in clients {
            let _ = client.shutdown();
        }

        self.clients.write().clear();
        self.clients_by_name.write().clear();
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
