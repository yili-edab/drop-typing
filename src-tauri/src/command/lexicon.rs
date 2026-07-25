//! 语音指令词表（M4）：别名 / 修饰词 / 键名 / 停用词 / 字母谐音。
//!
//! 词表分为"内置条目"和"用户条目"两部分。`Lexicon::build()` 在启动时
//! 将用户条目排在前面优先匹配，内置条目兜底，实现用户可自定义覆盖。

use crate::config::CommandConfig;

use super::Modifier;

// ---------- Owned 词表条目 ----------

/// Owned 版本的词表条目（替代旧的 `Lex`，字段使用 String/Vec 以支持运行时构建）。
#[derive(Debug, Clone)]
pub(super) enum LexOwned {
    Action(Vec<Modifier>, String),
    Mod(Modifier),
    Key(String),
    Stop,
}

/// 运行时词表：内置 + 用户条目合并后的可查询结构。
#[derive(Debug, Clone)]
pub struct Lexicon {
    /// 主词表条目（用户条目在前，内置在后）
    pub(super) main: Vec<(String, LexOwned)>,
    /// 谐音表：同样存为 LexOwned::Key 以统一 lookup_prefix 返回类型
    pub(super) homophones: Vec<(String, LexOwned)>,
}

impl Default for Lexicon {
    fn default() -> Self {
        Self::build(None)
    }
}

impl Lexicon {
    /// 从用户配置构建运行时词表。先推用户条目（优先匹配），再推内置（兜底）。
    pub fn build(user_cfg: Option<&CommandConfig>) -> Self {
        let mut main: Vec<(String, LexOwned)> = Vec::new();
        let mut homophones: Vec<(String, LexOwned)> = Vec::new();

        // 1. 用户条目在前（优先）
        if let Some(cfg) = user_cfg {
            add_user_entries(&mut main, &mut homophones, cfg);
        }

        // 2. 内置条目兜底
        add_builtin_main(&mut main);
        add_builtin_homophones(&mut homophones);

        Lexicon { main, homophones }
    }
}

// ---------- 用户条目追加 ----------

fn add_user_entries(
    main: &mut Vec<(String, LexOwned)>,
    homophones: &mut Vec<(String, LexOwned)>,
    cfg: &CommandConfig,
) {
    for a in &cfg.action {
        main.push((
            a.phrase.clone(),
            LexOwned::Action(a.modifiers.clone(), a.key.clone()),
        ));
    }
    for m in &cfg.modifier {
        main.push((m.phrase.clone(), LexOwned::Mod(m.modifier)));
    }
    for k in &cfg.key {
        main.push((k.phrase.clone(), LexOwned::Key(k.name.clone())));
    }
    for s in &cfg.stop {
        main.push((s.phrase.clone(), LexOwned::Stop));
    }
    for h in &cfg.homophone {
        homophones.push((
            h.phrase.clone(),
            LexOwned::Key(h.letter.clone()),
        ));
    }
}

// ---------- 内置词表 ----------

fn add_builtin_main(entries: &mut Vec<(String, LexOwned)>) {
    let cmd = vec![Modifier::Command];
    let shift_cmd = vec![Modifier::Shift, Modifier::Command];

    entries.extend([
        // ---- 动作别名 ----
        ("复制".into(), LexOwned::Action(cmd.clone(), "C".into())),
        ("拷贝".into(), LexOwned::Action(cmd.clone(), "C".into())),
        ("copy".into(), LexOwned::Action(cmd.clone(), "C".into())),
        ("粘贴".into(), LexOwned::Action(cmd.clone(), "V".into())),
        ("黏贴".into(), LexOwned::Action(cmd.clone(), "V".into())),
        ("paste".into(), LexOwned::Action(cmd.clone(), "V".into())),
        ("剪切".into(), LexOwned::Action(cmd.clone(), "X".into())),
        ("cut".into(), LexOwned::Action(cmd.clone(), "X".into())),
        ("撤销".into(), LexOwned::Action(cmd.clone(), "Z".into())),
        ("undo".into(), LexOwned::Action(cmd.clone(), "Z".into())),
        ("重做".into(), LexOwned::Action(shift_cmd.clone(), "Z".into())),
        ("redo".into(), LexOwned::Action(shift_cmd, "Z".into())), // shift_cmd moved
        ("全选".into(), LexOwned::Action(cmd.clone(), "A".into())),
        ("保存".into(), LexOwned::Action(cmd.clone(), "S".into())),
        ("save".into(), LexOwned::Action(cmd, "S".into())), // cmd moved
        // ---- 修饰词 ----
        ("shift".into(), LexOwned::Mod(Modifier::Shift)),
        ("换挡".into(), LexOwned::Mod(Modifier::Shift)),
        ("上档".into(), LexOwned::Mod(Modifier::Shift)),
        ("command".into(), LexOwned::Mod(Modifier::Command)),
        ("cmd".into(), LexOwned::Mod(Modifier::Command)),
        ("meta".into(), LexOwned::Mod(Modifier::Command)),
        ("命令".into(), LexOwned::Mod(Modifier::Command)),
        ("control".into(), LexOwned::Mod(Modifier::Control)),
        ("ctrl".into(), LexOwned::Mod(Modifier::Control)),
        ("控制".into(), LexOwned::Mod(Modifier::Control)),
        ("option".into(), LexOwned::Mod(Modifier::Option)),
        ("opt".into(), LexOwned::Mod(Modifier::Option)),
        ("alt".into(), LexOwned::Mod(Modifier::Option)),
        ("选项".into(), LexOwned::Mod(Modifier::Option)),
        // ---- 键名 ----
        ("enter".into(), LexOwned::Key("ENTER".into())),
        ("return".into(), LexOwned::Key("ENTER".into())),
        ("回车".into(), LexOwned::Key("ENTER".into())),
        ("换行".into(), LexOwned::Key("ENTER".into())),
        ("确认".into(), LexOwned::Key("ENTER".into())),
        ("发送".into(), LexOwned::Key("ENTER".into())),
        ("space".into(), LexOwned::Key("SPACE".into())),
        ("空格".into(), LexOwned::Key("SPACE".into())),
        ("tab".into(), LexOwned::Key("TAB".into())),
        ("制表".into(), LexOwned::Key("TAB".into())),
        ("esc".into(), LexOwned::Key("ESC".into())),
        ("escape".into(), LexOwned::Key("ESC".into())),
        ("逃逸".into(), LexOwned::Key("ESC".into())),
        ("delete".into(), LexOwned::Key("DELETE".into())),
        ("backspace".into(), LexOwned::Key("DELETE".into())),
        ("退格".into(), LexOwned::Key("DELETE".into())),
        ("删除".into(), LexOwned::Key("DELETE".into())),
        ("up".into(), LexOwned::Key("UP".into())),
        ("down".into(), LexOwned::Key("DOWN".into())),
        ("left".into(), LexOwned::Key("LEFT".into())),
        ("right".into(), LexOwned::Key("RIGHT".into())),
        ("方向键上".into(), LexOwned::Key("UP".into())),
        ("方向键下".into(), LexOwned::Key("DOWN".into())),
        ("方向键左".into(), LexOwned::Key("LEFT".into())),
        ("方向键右".into(), LexOwned::Key("RIGHT".into())),
        ("方向上".into(), LexOwned::Key("UP".into())),
        ("方向下".into(), LexOwned::Key("DOWN".into())),
        ("方向左".into(), LexOwned::Key("LEFT".into())),
        ("方向右".into(), LexOwned::Key("RIGHT".into())),
        ("上箭头".into(), LexOwned::Key("UP".into())),
        ("下箭头".into(), LexOwned::Key("DOWN".into())),
        ("左箭头".into(), LexOwned::Key("LEFT".into())),
        ("右箭头".into(), LexOwned::Key("RIGHT".into())),
        ("f1".into(), LexOwned::Key("F1".into())),
        ("f2".into(), LexOwned::Key("F2".into())),
        ("f3".into(), LexOwned::Key("F3".into())),
        ("f4".into(), LexOwned::Key("F4".into())),
        ("f5".into(), LexOwned::Key("F5".into())),
        ("f6".into(), LexOwned::Key("F6".into())),
        ("f7".into(), LexOwned::Key("F7".into())),
        ("f8".into(), LexOwned::Key("F8".into())),
        ("f9".into(), LexOwned::Key("F9".into())),
        ("f10".into(), LexOwned::Key("F10".into())),
        ("f11".into(), LexOwned::Key("F11".into())),
        ("f12".into(), LexOwned::Key("F12".into())),
        // 中文数字（优先于谐音表："一" 是 1 而不是 E）
        ("一".into(), LexOwned::Key("1".into())),
        ("二".into(), LexOwned::Key("2".into())),
        ("三".into(), LexOwned::Key("3".into())),
        ("四".into(), LexOwned::Key("4".into())),
        ("五".into(), LexOwned::Key("5".into())),
        ("六".into(), LexOwned::Key("6".into())),
        ("七".into(), LexOwned::Key("7".into())),
        ("八".into(), LexOwned::Key("8".into())),
        ("九".into(), LexOwned::Key("9".into())),
        ("零".into(), LexOwned::Key("0".into())),
        // ---- 停用词（填充词 / 连接词）----
        ("按一下".into(), LexOwned::Stop),
        ("按下".into(), LexOwned::Stop),
        ("按".into(), LexOwned::Stop),
        ("帮我".into(), LexOwned::Stop),
        ("请".into(), LexOwned::Stop),
        ("按键".into(), LexOwned::Stop),
        ("按钮".into(), LexOwned::Stop),
        ("键".into(), LexOwned::Stop),
        ("一下".into(), LexOwned::Stop),
        ("来".into(), LexOwned::Stop),
        ("个".into(), LexOwned::Stop),
        ("输入".into(), LexOwned::Stop),
        ("执行".into(), LexOwned::Stop),
        ("模拟".into(), LexOwned::Stop),
        ("组合键".into(), LexOwned::Stop),
        ("组合".into(), LexOwned::Stop),
        ("加".into(), LexOwned::Stop),
        ("和".into(), LexOwned::Stop),
        ("与".into(), LexOwned::Stop),
        ("杠".into(), LexOwned::Stop),
        ("plus".into(), LexOwned::Stop),
    ]);
}

fn add_builtin_homophones(entries: &mut Vec<(String, LexOwned)>) {
    entries.extend([
        ("哎".into(), LexOwned::Key("A".into())),
        ("比".into(), LexOwned::Key("B".into())),
        ("逼".into(), LexOwned::Key("B".into())),
        ("西".into(), LexOwned::Key("C".into())),
        ("弟".into(), LexOwned::Key("D".into())),
        ("意".into(), LexOwned::Key("E".into())),
        ("依".into(), LexOwned::Key("E".into())),
        ("记".into(), LexOwned::Key("G".into())),
        ("爱".into(), LexOwned::Key("I".into())),
        ("勾".into(), LexOwned::Key("J".into())),
        ("凯".into(), LexOwned::Key("K".into())),
        ("艾姆".into(), LexOwned::Key("M".into())),
        ("恩".into(), LexOwned::Key("N".into())),
        ("欧".into(), LexOwned::Key("O".into())),
        ("批".into(), LexOwned::Key("P".into())),
        ("球".into(), LexOwned::Key("Q".into())),
        ("阿尔".into(), LexOwned::Key("R".into())),
        ("丝".into(), LexOwned::Key("S".into())),
        ("替".into(), LexOwned::Key("T".into())),
        ("优".into(), LexOwned::Key("U".into())),
        ("威".into(), LexOwned::Key("V".into())),
        ("微".into(), LexOwned::Key("V".into())),
        ("达不溜".into(), LexOwned::Key("W".into())),
        ("叉".into(), LexOwned::Key("X".into())),
        ("歪".into(), LexOwned::Key("Y".into())),
        ("贼".into(), LexOwned::Key("Z".into())),
    ]);
}
