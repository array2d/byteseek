//! rwir `shell.run(sid)`：跑 `action/arg` 里的 bash，输出回填进 msg。

use crate::engine::Engine;

pub fn run(eng: &Engine, sid: &str) {
    super::tool_run(eng, sid, "shell", "bash", "-c");
}
