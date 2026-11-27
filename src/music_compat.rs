//! 音乐播放器兼容状态：每个播放器的版本，以及进度、时长、跳转等每一项能不能用、为什么不能用。
//! 读取模块写入，岛上据此决定是否显示歌词、是否提醒更新播放器；也负责保存「自动适配」学到的读取位置。
use std::{collections::HashMap,sync::Mutex};

/// 播放器的一项能力。
#[derive(Clone,Copy,PartialEq,Eq,Hash,Debug)]
pub enum Item {
    /// 播放进度（歌词同步依赖它）。
    Progress,
    /// 歌曲时长。
    Duration,
    /// 拖动进度条跳转。
    Seek,
    /// 播放器本机数据整体（波点、酷我：歌名、进度、歌词都从这里来）。
    Reader,
}

#[derive(Clone,PartialEq,Debug)]
pub enum Support {
    /// 已适配（版本在适配表里）。
    Ok,
    /// 不在适配表里，但已按数据特征自动找到。
    Learned,
    /// 正在按数据特征自动适配（需要播放几秒）。
    Learning,
    /// 不可用，附原因。
    Unavailable(String),
}

#[derive(Clone,Debug)]
pub struct PlayerStatus {
    pub exe:&'static str,
    pub version:Option<String>,
    pub items:Vec<(Item,Support)>,
}
impl PlayerStatus {
    pub fn get(&self,item:Item)->Option<&Support> {self.items.iter().find(|(i,_)|*i==item).map(|(_,s)|s)}
}

pub fn display_name(exe:&str)->&'static str {
    match exe.to_ascii_lowercase().as_str() {
        "kugou.exe"=>"酷狗音乐","cloudmusic.exe"=>"网易云音乐","bodian_pc.exe"=>"波点音乐","kwmusic.exe"=>"酷我音乐",
        "qqmusic.exe"=>"QQ 音乐","sodamusic.exe"=>"汽水音乐","spotify.exe"=>"Spotify","applemusic.exe"=>"Apple Music",_=>"播放器",
    }
}

static PLAYERS:Mutex<Vec<PlayerStatus>>=Mutex::new(Vec::new());

fn with_player<T>(exe:&'static str,f:impl FnOnce(&mut PlayerStatus)->T)->T {
    let mut players=PLAYERS.lock().unwrap_or_else(|e|e.into_inner());
    let index=match players.iter().position(|p|p.exe==exe) {
        Some(index)=>index,
        None=>{players.push(PlayerStatus{exe,version:None,items:Vec::new()});players.len()-1}
    };
    f(&mut players[index])
}

pub fn set_version(exe:&'static str,version:&str) {with_player(exe,|p|p.version=Some(version.to_owned()));}

pub fn set(exe:&'static str,item:Item,support:Support) {
    with_player(exe,|p|match p.items.iter_mut().find(|(i,_)|*i==item) {
        Some((_,old))=>*old=support,
        None=>p.items.push((item,support)),
    });
}

/// 播放器退出或重启后，旧进程的结论作废。
pub fn clear(exe:&'static str,item:Item) {
    let mut players=PLAYERS.lock().unwrap_or_else(|e|e.into_inner());
    if let Some(player)=players.iter_mut().find(|p|p.exe==exe){player.items.retain(|(i,_)|*i!=item);}
}

pub fn get(exe:&str)->Option<PlayerStatus> {
    PLAYERS.lock().unwrap_or_else(|e|e.into_inner()).iter().find(|p|p.exe.eq_ignore_ascii_case(exe)).cloned()
}

// ---- 版本较旧：岛上提醒更新一次 ----

/// 待显示的更新提醒，以及已经提醒过的「播放器/版本」（同一版本只提醒一次，重启后也不再提醒）。
static NOTICES:Mutex<(Vec<&'static str>,Option<std::collections::HashSet<String>>)>=Mutex::new((Vec::new(),None));

fn notices_path()->std::path::PathBuf {crate::model::config_dir().join("music-update-notices.txt")}

/// 播放器版本比已适配的还旧，导致歌词无法同步：排一条更新提醒。比已适配的新则是岛仔还没适配，不打扰用户。
pub fn outdated(exe:&'static str) {
    let version=get(exe).and_then(|p|p.version).unwrap_or_default();
    let key=format!("{exe}/{version}");
    let mut notices=NOTICES.lock().unwrap_or_else(|e|e.into_inner());
    let shown=notices.1.get_or_insert_with(||{
        if cfg!(test) {return Default::default();}
        std::fs::read_to_string(notices_path()).unwrap_or_default().lines().take(256).map(str::to_owned).collect()
    });
    if !shown.insert(key) {return;}
    if !cfg!(test) {
        let path=notices_path();
        if let Some(parent)=path.parent(){let _=std::fs::create_dir_all(parent);}
        let _=std::fs::write(path,shown.iter().cloned().collect::<Vec<_>>().join("\n"));
    }
    if !notices.0.contains(&exe) {notices.0.push(exe);}
}

/// 界面每帧取一次：有待显示的提醒时返回提示语。
pub fn take_update_notice()->Option<String> {
    let mut notices=NOTICES.lock().unwrap_or_else(|e|e.into_inner());
    if notices.0.is_empty() {return None;}
    let name=display_name(notices.0.remove(0));
    Some(format!("{name}版本较旧，更新后可同步歌词"))
}

/// 当前播放源的歌词同步是否受版本限制（进度或本机数据不可用）：岛上据此给出提示。
pub fn lyrics_limited(source:&str)->bool {
    let Some(exe)=super::music_log::player_exe(source) else{return false;};
    get(exe).is_some_and(|p|[Item::Progress,Item::Reader].iter().any(|item|matches!(p.get(*item),Some(Support::Unavailable(_)))))
}

// ---- 自动适配学到的读取位置：按组件文件名、PE 时间戳与映像大小区分版本，保存在配置目录 ----

fn store_path()->std::path::PathBuf {crate::model::config_dir().join("music-compat.json")}
fn key(module:&str,stamp:u32,size:u32)->String {format!("{}/{stamp:08x}/{size}",module.to_ascii_lowercase())}

static LEARNED:Mutex<Option<HashMap<String,usize>>>=Mutex::new(None);

fn load()->HashMap<String,usize> {
    if cfg!(test) {return HashMap::new();}
    let Ok(bytes)=std::fs::read(store_path()) else{return HashMap::new();};
    if bytes.len()>64*1024 {return HashMap::new();}
    serde_json::from_slice::<HashMap<String,usize>>(&bytes).unwrap_or_default()
}

fn save(map:&HashMap<String,usize>) {
    if cfg!(test) {return;}
    if let Ok(text)=serde_json::to_string_pretty(map) {
        let path=store_path();
        if let Some(parent)=path.parent(){let _=std::fs::create_dir_all(parent);}
        let _=std::fs::write(path,text);
    }
}

pub fn learned(module:&str,stamp:u32,size:u32)->Option<usize> {
    let mut store=LEARNED.lock().unwrap_or_else(|e|e.into_inner());
    store.get_or_insert_with(load).get(&key(module,stamp,size)).copied()
}

pub fn remember(module:&str,stamp:u32,size:u32,offset:usize) {
    let mut store=LEARNED.lock().unwrap_or_else(|e|e.into_inner());
    let map=store.get_or_insert_with(load);
    map.insert(key(module,stamp,size),offset);save(map);
}

pub fn forget(module:&str,stamp:u32,size:u32) {
    let mut store=LEARNED.lock().unwrap_or_else(|e|e.into_inner());
    let map=store.get_or_insert_with(load);
    if map.remove(&key(module,stamp,size)).is_some(){save(map);}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn items_are_tracked_per_player_and_limit_lyrics_only_for_progress_or_reader(){
        set("KuGou.exe",Item::Seek,Support::Unavailable("接口不可用".into()));
        assert!(!lyrics_limited("kugou"),"跳转不可用不影响歌词");
        set("KuGou.exe",Item::Progress,Support::Learning);
        assert!(!lyrics_limited("kugou"),"自动适配中先不提示");
        set("KuGou.exe",Item::Progress,Support::Unavailable("版本不匹配".into()));
        assert!(lyrics_limited("kugou"));
        set("KuGou.exe",Item::Progress,Support::Learned);
        assert!(!lyrics_limited("kugou"));
        assert!(get("kugou.exe").unwrap().get(Item::Seek).is_some_and(|s|matches!(s,Support::Unavailable(_))));
        clear("KuGou.exe",Item::Seek);
        assert!(get("KuGou.exe").unwrap().get(Item::Seek).is_none());
        assert!(!lyrics_limited("chrome.exe"));
    }
    #[test]
    fn outdated_players_are_reminded_once_per_version(){
        set_version("kwmusic.exe","9.2.0");
        outdated("kwmusic.exe");outdated("kwmusic.exe");
        assert_eq!(take_update_notice().as_deref(),Some("酷我音乐版本较旧，更新后可同步歌词"));
        assert_eq!(take_update_notice(),None,"同一版本只提醒一次");
        set_version("kwmusic.exe","9.3.0");outdated("kwmusic.exe");
        assert!(take_update_notice().is_some(),"换了版本仍旧较旧，再提醒一次");
    }

    #[test]
    fn learned_offsets_are_keyed_by_module_version(){
        remember("KGPlayer.dll",0x1234,4096,0x3949f8);
        assert_eq!(learned("kgplayer.dll",0x1234,4096),Some(0x3949f8));
        assert_eq!(learned("kgplayer.dll",0x1235,4096),None,"其他版本不能借用");
        forget("kgplayer.dll",0x1234,4096);
        assert_eq!(learned("kgplayer.dll",0x1234,4096),None);
    }
}
