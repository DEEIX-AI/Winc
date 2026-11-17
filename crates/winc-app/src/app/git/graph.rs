//! 提交视图底部的提交图：最近若干个提交连成的图，点一行打开该提交。
//!
//! 与 Git 日志标签里的提交图用的是同一套车道算法（[`winc_core::graph`]），这里只取最近
//! [`GRAPH_COMMITS`] 个提交，一次读完，不随日志标签滚动、筛选。`HEAD`、分支或点击刷新后重新
//! 读取（见 [`App::reload_graph`]）；读取期间旧内容留着，避免图闪一下。

use winc_core::graph::{Graph, GraphRow};
use winc_editor::git::{Commit, LogQuery};

use super::log::Loaded;
use super::unusable;
use crate::app::{App, Rect, UserEvent};

/// 提交视图里的提交图取多少个提交（个）。
///
/// 侧栏的图是给「最近做了什么」看的，与日志标签不同，不必读很深；一屏放不下时可以滚动。
pub(crate) const GRAPH_COMMITS: usize = 60;

/// 提交视图里的提交图。
#[derive(Debug, Default)]
pub(crate) struct GraphPart {
    /// 取回来的提交，最近的在前。
    pub commits: Vec<Commit>,
    /// 每一行怎么画，与 `commits` 一一对应。
    pub rows: Vec<GraphRow>,
    /// 列表滚动了多少（逻辑像素）。
    pub scroll: f32,
    /// 正在读取。
    pub loading: bool,
    /// 读取失败的原因。
    pub error: Option<String>,
    /// 当前这批的版本号：发出请求与返回不一致时丢弃结果。
    generation: u64,
}

impl GraphPart {
    /// 开始重新读取，返回这一批的版本号；旧内容留着，读到后整个换掉。
    fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.error = None;
        self.generation
    }

    /// 有没有可以画的内容。
    pub(crate) fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    /// 第 `index` 个提交的提交号。
    pub(crate) fn oid(&self, index: usize) -> Option<&str> {
        self.commits.get(index).map(|c| c.oid.as_str())
    }
}

impl App {
    /// 重新读取提交视图里的提交图（`HEAD` 变化、点击刷新、切换到提交视图时）。
    ///
    /// 正在读取时不再发出请求：当前这一批读完后按那时的 `HEAD` 生效。
    pub(in crate::app) fn reload_graph(&mut self) {
        let Some(root) = self.git.root.clone() else {
            self.git.graph = GraphPart::default();
            return;
        };
        if self.git.graph.loading {
            return;
        }
        let generation = self.git.graph.begin();
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let read = |git: winc_editor::git::Git| {
                let mut stream = git.log_stream(&root, &LogQuery::default())?;
                stream.next(GRAPH_COMMITS)
            };
            let result = tool.map_err(|e| unusable(&e)).and_then(read);
            let _ = sender.send(Loaded::Graph { generation, result });
        });
    }

    /// 后台读回的提交图（见 [`Loaded::Graph`]）：算好每行的车道后换上。
    pub(in crate::app) fn graph_arrived(
        &mut self,
        generation: u64,
        result: Result<Vec<Commit>, String>,
    ) {
        // 已经重新发起读取时丢下过时的结果。
        if generation != self.git.graph.generation {
            return;
        }
        self.git.graph.loading = false;
        match result {
            Ok(commits) => {
                let mut lanes = Graph::new();
                self.git.graph.rows = commits
                    .iter()
                    .map(|commit| lanes.push(&commit.oid, &commit.parents))
                    .collect();
                self.git.graph.commits = commits;
                self.git.graph.error = None;
                self.git.graph.scroll = self.git.graph.scroll.max(0.0);
            }
            Err(error) => self.git.graph.error = Some(error),
        }
        self.request_redraw();
    }

    /// 点中提交图里的第 `index` 行：在 Git 日志标签中打开这个提交。
    pub(in crate::app) fn press_graph_row(&mut self, index: usize) {
        let Some(oid) = self.git.graph.oid(index) else {
            return;
        };
        let oid = oid.to_owned();
        self.show_log_commit(&oid, false);
    }

    /// 提交图内容的高度下界、上界，以及改动列表至少留多高（逻辑像素）。
    ///
    /// 侧栏太矮时不显示提交图，改动列表优先。
    pub(crate) fn graph_section_height(&self, room: f32) -> Option<f32> {
        if self.git.graph.is_empty() || room < GRAPH_MIN_ROOM + GRAPH_MIN {
            return None;
        }
        Some(GRAPH_HEIGHT.min(room - GRAPH_MIN_ROOM))
    }

    /// 提交图最多能滚动多远（逻辑像素）：行数乘以行高，减去 `area` 里看得到的那部分。
    pub(crate) fn graph_scroll_max(&self, area: Rect) -> f32 {
        let rows = self.git.graph.commits.len() as f32;
        let room = (area.h - GRAPH_HEADER).max(0.0);
        (rows * GRAPH_ROW - room).max(0.0)
    }
}

/// 提交图占的高度（逻辑像素）：一行标题加八行提交。
///
/// 固定高度而不是按比例：提交信息框长高、储藏展开时改动列表照常伸缩，提交图不跟着变，
/// 列表里哪一行在什么位置也就不会跳。
const GRAPH_HEIGHT: f32 = 184.0;
/// 改动列表至少留这么高，放不下提交图时（窗口很矮）不显示提交图（逻辑像素）。
const GRAPH_MIN_ROOM: f32 = 140.0;
/// 提交图自己至少要这么高才显示（逻辑像素）：再矮就只剩标题，不如不显示。
const GRAPH_MIN: f32 = 120.0;
/// 提交图顶部标题行的高度（逻辑像素）。
pub(crate) const GRAPH_HEADER: f32 = 24.0;
/// 提交图一行的行高（逻辑像素）。
pub(crate) const GRAPH_ROW: f32 = 20.0;
