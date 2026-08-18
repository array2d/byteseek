//! rwir `agent.spawn(sid)`：把 `action/arg` 当子任务，新建子 session + 新 vthread，
//! 跑同一段 agentloop（独立 session、独立轮数额度、进度各自可观测），最终答案摘要回填父 msg。

use crate::engine::Engine;

pub fn spawn(eng: &Engine, sid: &str) {
    let subtask = eng.get_kv(&format!("/session/{sid}/action/arg"));
    let k = eng.subs.get() + 1;
    eng.subs.set(k);
    let sub = format!("sub{k}");
    println!("\n🤖 [{sid}] 派生子 agent {sub}: {subtask}");
    eng.seed_session(&sub, &subtask);
    eng.mkindex(&format!("/session/{sub}"));
    // 父的 sid 在 agentloop 入口只读一次并存进帧槽，故 cursid 被子改写不影响父。
    eng.run_entry(&sub);
    let summary = eng.get_kv(&format!("/session/{sub}/final"));
    let summary = if summary.is_empty() {
        "(子 agent 无最终答案)".into()
    } else {
        summary
    };
    println!("↳ 子 agent {sub} 摘要: {summary}");
    eng.append_msg(
        sid,
        "user",
        &format!("子 agent({subtask}) 返回:\n{summary}"),
    );
}
