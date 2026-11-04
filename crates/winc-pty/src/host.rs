//! 主程序这边：把按键和粘贴编码成写给伪终端的字节（[`input`]），启动辅助进程并收发消息
//! （[`client`]）。

pub mod client;
pub mod input;
