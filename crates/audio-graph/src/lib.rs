//! audio-graph：定长 DSP 处理器串联图。
//! audio-graph: fixed-length DSP processor pipeline graph.
//!
//! 第一阶段实现最简的**顺序处理链**（单声道/立体声，按帧数对齐），为后续
//! 插件图（第二阶段的任意 DAG）预留 `Node` 抽象边界。
//! Phase 1 implements the simplest **sequential processing chain** (mono/stereo,
//! frame-aligned), reserving the `Node` abstraction boundary for a future plugin
//! graph (arbitrary DAG in phase 2).
//!
//! 实时路径约束：图自身不分配（`process` 只遍历预分配的节点列表）。
//! Real-time constraint: the graph itself allocates nothing (`process` only walks
//! the pre-allocated node list).

#![forbid(unsafe_code)]

use dsp::FixedProcessor;

/// 图中的处理节点。
/// A processing node in the graph.
///
/// 第一阶段只有「处理器」一种节点类型；后续可扩展「分支/合并/输入/输出」。
/// Phase 1 has only a "processor" node kind; future kinds (branch/merge/input/output)
/// can be added later.
pub enum Node {
    /// 定长 DSP 处理器。A fixed-length DSP processor.
    Processor(Box<dyn FixedProcessor>),
    // 预留：Branch、Merge、Source、Sink 等（第二阶段）。
    // Reserved: Branch, Merge, Source, Sink, etc. (phase 2).
}

/// 顺序处理链：输入 → node[0] → node[1] → ... → 输出。
/// A sequential processing chain: input → node[0] → node[1] → ... → output.
///
/// 采用双缓冲（`scratch_a`/`scratch_b` 交替），避免每节点分配。
/// Uses double buffering (`scratch_a`/`scratch_b` alternating) to avoid per-node
/// allocation.
pub struct ProcessingGraph {
    /// 顺序节点列表。The ordered node list.
    nodes: Vec<Node>,
    /// 第一块工作缓冲区。First scratch buffer.
    scratch_a: Vec<f32>,
    /// 第二块工作缓冲区。Second scratch buffer.
    scratch_b: Vec<f32>,
    /// 一次 `process` 能处理的最大帧数（frames × channels）。
    /// Maximum sample count (frames × channels) one `process` can handle.
    capacity_samples: usize,
    /// 整条链的固定延迟（采样数）。Fixed latency of the whole chain (in samples).
    latency: usize,
}

impl ProcessingGraph {
    /// 创建空图。`capacity_samples` 应不小于最大 buffer 的 `frames × channels`。
    pub fn new(capacity_samples: usize) -> Self {
        Self {
            nodes: Vec::new(),
            scratch_a: vec![0.0; capacity_samples],
            scratch_b: vec![0.0; capacity_samples],
            capacity_samples,
            latency: 0,
        }
    }

    /// 追加一个处理器（实时路径建立前调用，运行中禁止修改）。
    pub fn push_processor(&mut self, p: Box<dyn FixedProcessor>) {
        self.latency += p.latency_samples();
        self.nodes.push(Node::Processor(p));
    }

    /// 清空图并重置所有处理器。
    pub fn reset(&mut self) {
        for n in &mut self.nodes {
            match n {
                Node::Processor(p) => p.reset(),
            }
        }
    }

    /// 图的固定延迟（采样数）。
    pub fn latency_samples(&self) -> usize {
        self.latency
    }

    /// 按参数 ID 设置图中某个处理器的参数（逐节点查找）。
    ///
    /// 出于实时安全，这里仅对**带名字匹配**的处理器生效；引擎侧用它做参数
    /// 下发。运行中调用需保证该处理器已接入。
    pub fn set_param_by_name(&mut self, name: &str, id: u32, value: f32) {
        for n in &mut self.nodes {
            match n {
                Node::Processor(p) => {
                    if p.name() == name {
                        p.set_param(id, value);
                    }
                }
            }
        }
    }

    /// 处理一批帧。`frames × channels <= capacity_samples`。
    ///
    /// 输入被第一节点读取；输出写入 `output`。
    pub fn process(&mut self, input: &[f32], output: &mut [f32], frames: usize, channels: usize) {
        let n = frames * channels;
        debug_assert!(n <= self.capacity_samples);

        if self.nodes.is_empty() {
            output[..n].copy_from_slice(&input[..n]);
            return;
        }

        // 第一个节点从 scratch_a 读取（已拷入 input）。
        // The first node reads from scratch_a (input already copied in).
        self.scratch_a[..n].copy_from_slice(&input[..n]);

        // 逐节点处理：偶数索引节点读 scratch_a 写 scratch_b，奇数反之；
        // 相邻节点 ping-pong 交替使用两块缓冲区，避免额外拷贝与 unsafe。
        // Process node by node: even-indexed nodes read scratch_a and write
        // scratch_b, odd nodes the reverse; adjacent nodes ping-pong between the
        // two buffers, avoiding extra copies and any unsafe code.
        let mut idx = 0usize;
        for node in &mut self.nodes {
            match node {
                Node::Processor(p) => {
                    let (src, dst) = if idx % 2 == 0 {
                        (self.scratch_a.as_mut_slice(), self.scratch_b.as_mut_slice())
                    } else {
                        (self.scratch_b.as_mut_slice(), self.scratch_a.as_mut_slice())
                    };
                    p.process(&src[..n], &mut dst[..n], frames);
                }
            }
            idx += 1;
        }

        // 最终结果在最后一个节点写的目标缓冲区。
        // The final result is in the buffer written by the last node.
        // 节点索引 i 的 dst：i 偶数 → scratch_b，i 奇数 → scratch_a。
        // Node i's dst: even i → scratch_b, odd i → scratch_a.
        // 最后一个节点索引 = idx-1。
        // Last node index = idx-1.
        let final_buf = if (idx.saturating_sub(1)) % 2 == 0 {
            &self.scratch_b
        } else {
            &self.scratch_a
        };
        output[..n].copy_from_slice(&final_buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsp::{Biquad, BiquadKind, BiquadParams, Gain};

    #[test]
    fn empty_graph_is_passthrough() {
        let mut g = ProcessingGraph::new(8);
        let input = [0.1, 0.2, 0.3, 0.4];
        let mut out = [0.0; 4];
        g.process(&input, &mut out, 2, 2);
        assert_eq!(out, input);
    }

    #[test]
    fn gain_node_scales() {
        let mut g = ProcessingGraph::new(8);
        g.push_processor(Box::new(Gain::new(-6.0)));
        let input = [1.0, 1.0, 1.0, 1.0];
        let mut out = [0.0; 4];
        g.process(&input, &mut out, 2, 2);
        // -6dB ≈ 0.5
        assert!((out[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn chained_nodes_compose() {
        let mut g = ProcessingGraph::new(8);
        g.push_processor(Box::new(Gain::new(0.0)));
        let p = BiquadParams::rbj(BiquadKind::LowPass, 48_000.0, 10_000.0, 0.707, 0.0);
        g.push_processor(Box::new(Biquad::new(p, 1)));
        let input = [0.5];
        let mut out = [0.0; 1];
        g.process(&input, &mut out, 1, 1);
        // 输出应为有限值
        assert!(out[0].is_finite());
    }
}
