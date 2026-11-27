//! 办公客户端只监听任务栏事件；唤起时仅处理已确认的原窗口。
use super::{Messenger,OpenError,Target,Unread};
use super::windows::*;
use windows::{core::Owned,Win32::{Foundation::{BOOL,HWND,LPARAM},System::Threading::{OpenProcess,PROCESS_QUERY_LIMITED_INFORMATION},UI::WindowsAndMessaging::*}};

pub(crate) const WECOM:NativeAdapter=NativeAdapter{
    paths:||paths(Messenger::WeCom),main_frame:|class,style,extended|main_frame(Messenger::WeCom,class,style,extended),
    accounts:None,ready,strict_window:true,open,taskbar:Some(|ctx|start(Messenger::WeCom,ctx)),
};
pub(crate) const FEISHU:NativeAdapter=NativeAdapter{
    paths:||paths(Messenger::Feishu),main_frame:|class,style,extended|main_frame(Messenger::Feishu,class,style,extended),
    accounts:None,ready,strict_window:true,open,taskbar:Some(|ctx|start(Messenger::Feishu,ctx)),
};
pub(crate) const DINGTALK:NativeAdapter=NativeAdapter{
    paths:||paths(Messenger::DingTalk),main_frame:|class,style,extended|main_frame(Messenger::DingTalk,class,style,extended),
    accounts:None,ready,strict_window:true,open,taskbar:Some(|ctx|start(Messenger::DingTalk,ctx)),
};
fn paths(client:Messenger)->Vec<String>{client.adapter().process_names.iter().map(|name|format!("*{}",name.to_ascii_lowercase())).collect()}
fn start(client:Messenger,ctx:eframe::egui::Context)->Result<Box<dyn TaskbarSource>,String>{
    Ok(Box::new(crate::wechat::Listener::for_client(client,ctx)?))
}
fn main_frame(client:Messenger,class:&str,style:u32,extended:u32)->bool{
    if style&WS_CHILD.0!=0 || extended&(WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE).0!=0{return false;}
    match client{
        Messenger::WeCom=>class=="WeWorkWindow",
        Messenger::Feishu=>class=="Chrome_WidgetWin_1" && frame_style(class,style,extended),
        Messenger::DingTalk=>(class=="StandardFrame_DingTalk" || class.starts_with("Qt") && class.ends_with("QWindowIcon")) && frame_style(class,style,extended),
        _=>false,
    }
}
pub(crate) unsafe fn main_window(window:HWND,client:Messenger)->bool{
    if !IsWindow(window).as_bool() || GetWindow(window,GW_OWNER).is_ok(){return false;}
    let class=class_name(window);
    let style=GetWindowLongW(window,GWL_STYLE)as u32;
    if !client_main_frame(client,&class,style,GetWindowLongW(window,GWL_EXSTYLE)as u32){return false;}
    if client==Messenger::DingTalk && class!=DINGTALK_FRAME{
        let pid=window_pid(window);
        if has_dingtalk_frame(pid) || !IsWindowVisible(window).as_bool()
            && has_preferred_dingtalk_qt(pid,dingtalk_frameless_main_style(style)){return false;}
    }
    if client==Messenger::Feishu && !feishu_title(&window_title(window)) && feishu_main_frame(window_pid(window)).is_some(){return false;}
    window_sized(window)
}
/// 最小化时按恢复尺寸检查，排除任何状态下不足 300×300 的小弹窗。
unsafe fn window_sized(window:HWND)->bool{
    use windows::Win32::Foundation::RECT;
    let mut rect=RECT::default();
    if IsIconic(window).as_bool(){
        let mut placement=WINDOWPLACEMENT{length:std::mem::size_of::<WINDOWPLACEMENT>() as u32,..Default::default()};
        if GetWindowPlacement(window,&mut placement).is_err(){return false;}
        rect=placement.rcNormalPosition;
    }else if GetWindowRect(window,&mut rect).is_err(){return false;}
    rect.right-rect.left>=300 && rect.bottom-rect.top>=300
}
const DINGTALK_FRAME:&str="StandardFrame_DingTalk";
/// 钉钉 9.0.5 的 Qt 主窗口没有系统标题栏，但有完整的窗口控制样式。同进程的隐藏对话框带标题栏。
fn dingtalk_frameless_main_style(style:u32)->bool{
    let frame=(WS_POPUP|WS_SYSMENU|WS_THICKFRAME|WS_MINIMIZEBOX|WS_MAXIMIZEBOX).0;
    style&(frame|WS_CAPTION.0)==frame
}
/// 可见窗口优先。全隐藏时，仍要用闪动窗口和同进程唯一候选同时确认主窗口。
unsafe fn has_preferred_dingtalk_qt(pid:u32,visible_only:bool)->bool{
    struct Search{pid:u32,visible_only:bool,found:bool}
    unsafe extern "system" fn visit(window:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        if window_pid(window)==search.pid && GetWindow(window,GW_OWNER).is_err(){
            let class=class_name(window);
            let style=GetWindowLongW(window,GWL_STYLE)as u32;
            if class.starts_with("Qt") && class.ends_with("QWindowIcon")
                && (IsWindowVisible(window).as_bool() || !search.visible_only && dingtalk_frameless_main_style(style))
                && main_frame(Messenger::DingTalk,&class,style,GetWindowLongW(window,GWL_EXSTYLE)as u32)
                && window_sized(window){search.found=true;return false.into();}
        }
        true.into()
    }
    let mut search=Search{pid,visible_only,found:false};
    let _=EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize));
    search.found
}
/// 新版钉钉的主窗口是 StandardFrame_DingTalk；同进程里的 Qt 窗口只是对话框（常为隐藏状态），
/// 不能算作第二个主窗口。主窗口本身隐藏到托盘时仍然认它。只有旧版没有该窗口时才按 Qt 主窗口识别。
unsafe fn has_dingtalk_frame(pid:u32)->bool{
    struct Search{pid:u32,found:bool}
    unsafe extern "system" fn visit(window:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        if window_pid(window)==search.pid && class_name(window)==DINGTALK_FRAME && GetWindow(window,GW_OWNER).is_err()
            && main_frame(Messenger::DingTalk,DINGTALK_FRAME,GetWindowLongW(window,GWL_STYLE)as u32,GetWindowLongW(window,GWL_EXSTYLE)as u32)
            && window_sized(window){
            search.found=true;return false.into();
        }
        true.into()
    }
    let mut search=Search{pid,found:false};
    let _=EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize));
    search.found
}
/// 闪动没能对应到账号时写进诊断日志：同进程所有无所属窗口的顶层窗口，
/// 列出类名、样式、尺寸、可见状态和是否算主窗口，不读标题。最多列 12 个。
pub(crate) fn describe_process_windows(client:Messenger,pid:u32)->String{unsafe{
    use windows::Win32::Foundation::RECT;
    struct Search{pid:u32,windows:Vec<HWND>}
    unsafe extern "system" fn visit(window:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        if window_pid(window)==search.pid && GetWindow(window,GW_OWNER).is_err(){search.windows.push(window);}
        true.into()
    }
    let mut search=Search{pid,windows:Vec::new()};
    let _=EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize));
    let total=search.windows.len();
    let mut lines=search.windows.iter().take(12).map(|&window|{
        let mut rect=RECT::default();let _=GetWindowRect(window,&mut rect);
        format!("{:#x} {} 样式 {:#010x}/{:#x} {}×{}{}{}{}",window.0 as usize,class_name(window),
            GetWindowLongW(window,GWL_STYLE)as u32,GetWindowLongW(window,GWL_EXSTYLE)as u32,rect.right-rect.left,rect.bottom-rect.top,
            if IsWindowVisible(window).as_bool(){""}else{" 隐藏"},if IsIconic(window).as_bool(){" 最小化"}else{""},
            if main_window(window,client){" 主窗口"}else{""})
    }).collect::<Vec<_>>();
    if total>12{lines.push(format!("另有 {} 个",total-12));}
    format!("进程 {pid} 顶层窗口 {total} 个：{}",lines.join("；"))
}}
/// 飞书主窗口的标题固定为应用名（切换消息、云文档等页面不变，关到托盘后仍保留）；
/// 「在独立窗口打开」的会话、文档窗口与主窗口同进程、同类名、同样式，只有标题是会话或文档名。
const FEISHU_TITLES:[&str;3]=["飞书","Feishu","Lark"];
const FEISHU_FRAME:&str="Chrome_WidgetWin_1";
fn feishu_title(title:&str)->bool{FEISHU_TITLES.iter().any(|name|name.eq_ignore_ascii_case(title.trim()))}
fn window_title(window:HWND)->String{
    let mut text=[0u16;256];
    let n=unsafe{GetWindowTextW(window,&mut text)}.max(0) as usize;
    String::from_utf16_lossy(&text[..n])
}
unsafe fn feishu_frame(window:HWND)->bool{
    class_name(window)==FEISHU_FRAME && GetWindow(window,GW_OWNER).is_err()
        && main_frame(Messenger::Feishu,FEISHU_FRAME,GetWindowLongW(window,GWL_STYLE)as u32,GetWindowLongW(window,GWL_EXSTYLE)as u32)
}
/// 同进程里唯一一个以应用名为标题的飞书主窗口；没有或不止一个（比如有会话恰好叫「飞书」）时不猜。
unsafe fn feishu_main_frame(pid:u32)->Option<HWND>{
    struct Search{pid:u32,found:Vec<HWND>}
    unsafe extern "system" fn visit(window:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        if window_pid(window)==search.pid && feishu_frame(window) && feishu_title(&window_title(window))
            && window_sized(window){search.found.push(window);}
        true.into()
    }
    let mut search=Search{pid,found:Vec::new()};
    let _=EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize));
    match search.found.as_slice(){[one]=>Some(*one),_=>None}
}
/// 飞书独立会话 / 文档窗口闪动时，归到同进程的主窗口：同一进程就是同一个飞书实例，不会串到其他账号。
unsafe fn feishu_owner(window:HWND)->Option<HWND>{
    if !feishu_frame(window) || feishu_title(&window_title(window)){return None;}
    feishu_main_frame(window_pid(window))
}
fn candidates(client:Messenger,pid:u32)->Option<Vec<Target>>{unsafe{
    let process=Owned::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,pid).ok()?);
    struct Search{client:Messenger,pid:u32,windows:Vec<Unread>}
    unsafe extern "system" fn visit(window:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        if window_pid(window)==search.pid && main_window(window,search.client){
            search.windows.push(Unread{source:search.client,pid:search.pid,window:window.0 as isize});
        }
        true.into()
    }
    let mut search=Search{client,pid,windows:Vec::new()};
    EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize)).ok()?;
    snapshot_account(pid,search.windows,|_|process_started(*process))
}}
fn snapshot_account(pid:u32,windows:impl IntoIterator<Item=Unread>,started_at:impl FnOnce(u32)->Option<u64>)->Option<Vec<Target>>{
    let started=started_at(pid)?;
    Some(windows.into_iter().filter(|unread|unread.pid==pid).map(|unread|Target{unread,started}).collect())
}
/// 只接受原事件窗口或其明确的所属主窗口，同进程的其他窗口不能借用。
fn select_target(client:Messenger,window:isize,accounts:&[Target])->Option<Target>{
    let target=accounts.iter().find(|target|target.unread.source==client && target.unread.window==window)?;
    (accounts.iter().filter(|other|other.unread.source==client && other.unread.pid==target.unread.pid).count()==1).then_some(*target)
}
pub(crate) fn flash_account(client:Messenger,window:HWND,paths:&[String])->Option<Option<Target>>{unsafe{
    let source_pid=window_pid(window);
    if !pid_matches(source_pid,paths){return None;}
    let mut root=GetAncestor(window,GA_ROOTOWNER);
    if client==Messenger::Feishu {if let Some(main)=feishu_owner(root){root=main;}}
    let pid=window_pid(root);
    if pid!=source_pid && !pid_matches(pid,paths){return Some(None);}
    let Some(accounts)=candidates(client,pid)else{return Some(None);};
    if client==Messenger::DingTalk && root==window && accounts.is_empty() && dingtalk_login_window(window){return None;}
    Some(select_target(client,root.0 as isize,&accounts))
}}
#[cfg(test)]
pub(crate) fn flash_target(client:Messenger,window:HWND,paths:&[String])->Option<Unread>{
    flash_account(client,window,paths).map(|target|target.map_or_else(||Unread::of(client),|t|t.unread))
}
/// 钉钉 9.0.5 扫码前会闪动竖版登录窗口，登录成功后在同一进程中另建消息主窗口。
unsafe fn dingtalk_login_window(window:HWND)->bool{
    let class=class_name(window);
    let style=GetWindowLongW(window,GWL_STYLE) as u32;
    let fixed_popup=(WS_POPUP|WS_CLIPSIBLINGS|WS_CLIPCHILDREN|WS_MINIMIZEBOX).0;
    if !class.starts_with("Qt") || !class.ends_with("QWindowIcon") || GetWindowLongW(window,GWL_EXSTYLE)!=0
        || style & !(WS_VISIBLE|WS_MINIMIZE).0 != fixed_popup{return false;}
    let mut placement=WINDOWPLACEMENT{length:std::mem::size_of::<WINDOWPLACEMENT>() as u32,..Default::default()};
    if GetWindowPlacement(window,&mut placement).is_err(){return false;}
    let rect=placement.rcNormalPosition;
    let (width,height)=(i64::from(rect.right-rect.left),i64::from(rect.bottom-rect.top));
    // 实测宽高比 600:798，允许不同显示缩放带来的两个像素取整差。
    width>0 && (height*600-width*798).abs()<=1200
}
fn ready(target:Target)->bool{
    ready_with_paths(target,&client_paths(target.unread.source))
}
fn ready_with_paths(target:Target,paths:&[String])->bool{
    let pid=target.unread.pid;
    (unsafe{pid_matches(pid,paths)}) && matches!(candidates(target.unread.source,pid).as_deref(),Some([one]) if *one==target)
}
pub(super) fn matches_foreground(target:Target,window:HWND,paths:&[String])->bool{
    let window=window.0 as isize;
    if target.unread.window==window{return ready_with_paths(target,paths);}
    // 手动打开客户端时也只确认原进程中唯一合格的前台新主窗。
    rebuild_target(target,paths).is_some_and(|rebuilt|rebuilt.unread.window==window)
}
/// 重建时没有新闪动可校验，必须有该客户端明确的主窗口特征。
pub(crate) fn rebuild_target(target:Target,paths:&[String])->Option<Target>{unsafe{
    let pid=target.unread.pid;
    let window=HWND(target.unread.window as *mut _);
    if window_pid(window)==pid || !pid_matches(pid,paths){return None;}
    let accounts=candidates(target.unread.source,pid)?;
    let [one]=accounts.as_slice() else{return None;};
    if one.started!=target.started{return None;}
    let replacement=HWND(one.unread.window as *mut _);
    let trusted=match target.unread.source{
        Messenger::Feishu=>feishu_title(&window_title(replacement)),
        Messenger::DingTalk=>class_name(replacement)==DINGTALK_FRAME
            || dingtalk_frameless_main_style(GetWindowLongW(replacement,GWL_STYLE) as u32),
        Messenger::WeCom=>true,
        _=>false,
    };
    trusted.then_some(*one)
}}
fn open(target:Target,paths:&[String],_:bool)->Result<Target,OpenError>{unsafe{
    let unread=target.unread;
    let window=HWND(unread.window as *mut _);
    if unread.pid==0 || window_pid(window)!=unread.pid {wake_step("提醒窗口已不属于该账号");return Err(OpenError::Unavailable);}
    if !main_window(window,unread.source){wake_step(format!("提醒窗口不是主窗口（{}）",describe_window(window)));return Err(OpenError::Unavailable);}
    if !matches(window,paths){wake_step("提醒窗口所属程序不在已知安装路径中");return Err(OpenError::Unavailable);}
    // 不执行协议、托盘回调或鼠标键盘模拟，只恢复该消息保存的主窗口。
    wake_step(format!("切前台 {}",describe_window(window)));
    let show_queued=match wake_route(unread.source,IsWindowVisible(window).as_bool(),IsIconic(window).as_bool()) {
        WakeRoute::Restore=>Some(ShowWindowAsync(window,SW_RESTORE).as_bool()),
        WakeRoute::Show=>Some(ShowWindowAsync(window,SW_SHOW).as_bool()),
        WakeRoute::Relaunch=>return relaunch(target,paths),
        WakeRoute::Focus=>None,
    };
    finish_open(show_queued,||SetForegroundWindow(window).as_bool()).map(|_|target)
}}

fn finish_open(show_queued:Option<bool>,focus:impl FnOnce()->bool)->Result<(),OpenError>{
    if show_queued==Some(false){wake_step("系统未接受窗口恢复请求");return Err(OpenError::Unavailable);}
    if focus(){wake_step("系统已接受");return Ok(());}
    if show_queued==Some(true){
        wake_step("恢复请求已提交，等待原窗口到前台");
        Ok(())
    }else{
        wake_step("系统拒绝切前台");
        Err(OpenError::Unavailable)
    }
}

#[derive(Debug,PartialEq,Eq)]
enum WakeRoute {Focus,Restore,Show,Relaunch}
/// 怎样把主窗口带到前台。飞书关到托盘后窗口被隐藏，若由岛仔直接 `ShowWindow`，飞书不会恢复渲染，
/// 页面停在加载骨架、点击无响应（#181，本机飞书 8.6 复现）；改为再启动一次飞书主程序，由飞书自己显示原窗口。
/// 最小化的窗口照常还原（实测正常）；其他客户端未验证，保持原做法。
fn wake_route(source:Messenger,visible:bool,iconic:bool)->WakeRoute {
    if !visible && source==Messenger::Feishu {WakeRoute::Relaunch}
    else if iconic {WakeRoute::Restore}
    else if visible {WakeRoute::Focus}
    else {WakeRoute::Show}
}

/// 启动提醒窗口所属进程自己的程序（单实例：不会多开，飞书把已有主窗口显示出来），再等窗口出现。
unsafe fn relaunch(target:Target,paths:&[String])->Result<Target,OpenError>{
    let pid=target.unread.pid;
    let Some(path)=crate::smtc::player_process::image_path(pid) else{wake_step("无法读取客户端程序路径");return Err(OpenError::Unavailable);};
    if !path_allowed(&normalized(&path),paths) {wake_step("客户端程序不在已知安装路径中");return Err(OpenError::Unavailable);}
    super::trust::launch_feishu(std::path::Path::new(&path),pid).map_err(|reason|{
        wake_step(reason);OpenError::Failed(reason.into())
    })?;
    wake_step("窗口在托盘中，由客户端自己显示主窗口");
    let until=std::time::Instant::now()+std::time::Duration::from_secs(3);
    while std::time::Instant::now()<until {
        if let Some(shown)=visible_relaunched_target(target,paths){
            if shown==target{wake_step("客户端已显示原主窗口");}
            else{wake_step(format!("客户端已显示同一账号的新主窗口 · {:#x} → {:#x}",target.unread.window,shown.unread.window));}
            let window=HWND(shown.unread.window as *mut _);
            return finish_open(Some(true),||SetForegroundWindow(window).as_bool()).map(|_|shown);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    wake_step("客户端 3 秒内没有显示主窗口");Err(OpenError::Unavailable)
}

unsafe fn visible_relaunched_target(original:Target,paths:&[String])->Option<Target>{
    let window=HWND(original.unread.window as *mut _);
    let target=if window_pid(window)==original.unread.pid {original}else{rebuild_target(original,paths)?};
    let window=HWND(target.unread.window as *mut _);
    (window_pid(window)==target.unread.pid && IsWindowVisible(window).as_bool()
        && !IsIconic(window).as_bool() && main_window(window,target.unread.source)).then_some(target)
}

#[cfg(test)]
mod tests{
    use super::*;
    use windows::{core::w,Win32::{Foundation::{LRESULT,WPARAM},System::{LibraryLoader::GetModuleHandleW,Threading::GetCurrentProcess}}};
    fn target(client:Messenger,pid:u32,window:isize)->Target{Target{unread:Unread{source:client,pid,window},started:1}}
    #[test]
    fn three_or_more_instances_never_rebind_to_another_or_the_last_instance(){
        for client in Messenger::OFFICE{
            let (a,b,c)=(target(client,100,11),target(client,200,22),target(client,300,33));
            assert_eq!(select_target(client,11,&[a,b,c]),Some(a));
            assert_eq!(select_target(client,22,&[a,b,c]),Some(b));
            assert_eq!(select_target(client,11,&[b,c]),None);
            assert_eq!(select_target(client,11,&[c]),None);
            assert_eq!(select_target(client,11,&[a,Target{unread:Unread{window:44,..a.unread},..a}]),None,"同进程多窗口不能当作唯一账号");
        }
    }
    #[test]
    fn another_instance_exit_does_not_invalidate_the_original_candidate(){
        for client in [Messenger::DingTalk,Messenger::Feishu]{
            let original=target(client,100,11);
            let exited=target(client,200,22);
            let mut queried=Vec::new();
            let snapshot=snapshot_account(100,[exited.unread,original.unread],|pid|{
                queried.push(pid);
                (pid==100).then_some(original.started)
            });
            assert_eq!(snapshot,Some(vec![original]));
            assert_eq!(queried,[100],"另一实例退出时不应查询它的进程元数据");
        }
    }
    #[test]
    fn feishu_in_tray_is_shown_by_feishu_itself(){
        assert_eq!(wake_route(Messenger::Feishu,false,false),WakeRoute::Relaunch,"关到托盘的飞书不能由岛仔直接显示（#181 卡死）");
        assert_eq!(wake_route(Messenger::Feishu,false,true),WakeRoute::Relaunch,"隐藏优先于残留的最小化标志");
        assert_eq!(wake_route(Messenger::Feishu,true,true),WakeRoute::Restore,"最小化的飞书照常还原");
        assert_eq!(wake_route(Messenger::Feishu,true,false),WakeRoute::Focus);
        for client in [Messenger::DingTalk,Messenger::WeCom] {assert_eq!(wake_route(client,false,false),WakeRoute::Show,"其他客户端未验证，保持原做法");}
    }
    #[test]
    fn a_queued_window_restore_waits_for_foreground_confirmation(){
        assert_eq!(finish_open(Some(true),||false),Ok(()),"恢复请求已进入窗口队列，应继续等原窗口切到前台");
        assert_eq!(finish_open(Some(false),||panic!("恢复请求被拒后无需再尝试置前")),Err(OpenError::Unavailable));
        assert_eq!(finish_open(None,||false),Err(OpenError::Unavailable),"仅置前的请求被拒后没有待确认的恢复操作");
    }
    #[test]
    fn feishu_rebuilt_during_relaunch_waits_for_the_same_accounts_visible_main_window(){unsafe{
        unsafe extern "system" fn proc(h:HWND,m:u32,w:WPARAM,l:LPARAM)->LRESULT{DefWindowProcW(h,m,w,l)}
        let instance=GetModuleHandleW(None).unwrap().into();
        let class=w!("Chrome_WidgetWin_1");
        RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(proc),hInstance:instance,lpszClassName:class,..Default::default()});
        let create=|title|CreateWindowExW(WINDOW_EX_STYLE(0),class,title,WS_OVERLAPPEDWINDOW,-10000,-10000,500,500,None,None,instance,None).unwrap();
        let hidden=create(w!("飞书"));
        let original=Target{unread:Unread{source:Messenger::Feishu,pid:std::process::id(),window:hidden.0 as isize},
            started:process_started(GetCurrentProcess()).unwrap()};
        let paths=[normalized(&std::env::current_exe().unwrap().to_string_lossy())];
        assert_eq!(visible_relaunched_target(original,&paths),None);
        DestroyWindow(hidden).unwrap();
        assert_eq!(visible_relaunched_target(original,&paths),None,"新主窗口尚未创建时应继续等待");
        let helper=create(w!("独立会话"));
        let _=ShowWindow(helper,SW_SHOWNOACTIVATE);
        let unrelated=visible_relaunched_target(original,&paths);
        DestroyWindow(helper).unwrap();
        assert_eq!(unrelated,None,"同进程的独立窗口不能接管这次唤起");
        let replacement=create(w!("飞书"));
        assert_eq!(visible_relaunched_target(original,&paths),None,"新窗口隐藏时应继续等待");
        let _=ShowWindow(replacement,SW_SHOWNOACTIVATE);
        let shown=visible_relaunched_target(original,&paths);
        DestroyWindow(replacement).unwrap();
        assert_eq!(shown,Some(Target{unread:Unread{window:replacement.0 as isize,..original.unread},..original}));
    }}
    /// 本机实测：先把飞书关到托盘，再运行 `cargo test live_feishu -- --ignored`，走岛仔点提醒时的同一条唤起路径。
    #[test]
    #[ignore="需要本机飞书已登录并关到托盘"]
    fn live_feishu_tray_window_is_shown_by_feishu(){unsafe{
        let paths=client_paths(Messenger::Feishu);
        let accounts:Vec<_>=client_processes(Messenger::Feishu).expect("应找到飞书进程").into_iter()
            .filter_map(|pid|candidates(Messenger::Feishu,pid)).flatten().collect();
        let [target]=accounts.as_slice() else{panic!("应只有一个飞书主窗口：{}",accounts.len())};
        let window=HWND(target.unread.window as *mut _);
        assert!(!IsWindowVisible(window).as_bool(),"请先把飞书关到托盘");
        crate::messenger::windows::take_wake_steps();
        let result=open(*target,&paths,true);
        let steps=crate::messenger::windows::take_wake_steps();
        eprintln!("{steps:?}");
        assert_eq!(result,Ok(*target));
        let until=std::time::Instant::now()+std::time::Duration::from_secs(4);
        while GetForegroundWindow()!=window && std::time::Instant::now()<until{
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(GetForegroundWindow(),window,"应将原飞书主窗口切到前台");
        assert!(IsWindowVisible(window).as_bool() && !IsIconic(window).as_bool());
        assert!(steps.iter().any(|s|s.contains("由客户端自己显示")));
    }}
    #[test]
    fn main_window_rules_exclude_login_popups_and_cross_client_windows(){
        let frame=WS_OVERLAPPEDWINDOW.0;
        assert!(main_frame(Messenger::WeCom,"WeWorkWindow",WS_POPUP.0,0));
        assert!(main_frame(Messenger::Feishu,"Chrome_WidgetWin_1",frame,0));
        assert!(main_frame(Messenger::DingTalk,"Qt51511QWindowIcon",frame,0));
        assert!(main_frame(Messenger::DingTalk,"StandardFrame_DingTalk",0x360f0000,0x100),"支持实际日志中的已登录钉钉主窗口和最小化状态");
        for client in Messenger::OFFICE{
            assert!(!main_frame(client,"WeChatLogin",frame,0));
            assert!(!main_frame(client,"Chrome_WidgetWin_0",frame,0));
            assert!(!main_frame(client,"PerryShadowWnd",frame,0));
        }
        assert!(!main_frame(Messenger::Feishu,"Chrome_WidgetWin_1",WS_POPUP.0,0));
        assert!(!main_frame(Messenger::DingTalk,"Qt51511QWindowIcon",0x96020000,0));
        assert!(!main_frame(Messenger::DingTalk,"StandardFrame_DingTalk",WS_POPUP.0,0));
        assert!(!main_frame(Messenger::DingTalk,"StandardFrame_DingTalk",frame,WS_EX_TOOLWINDOW.0));
        assert!(!main_frame(Messenger::DingTalk,"StandardFrame_DingTalk",frame|WS_CHILD.0,0));
        assert!(!main_frame(Messenger::Feishu,"StandardFrame_DingTalk",frame,0));
        assert!(!main_frame(Messenger::WeCom,"WeWorkWindow",WS_POPUP.0,WS_EX_TOOLWINDOW.0));
    }
    #[test]
    fn synthetic_native_windows_keep_original_handles_and_reject_ambiguity(){unsafe{
        unsafe extern "system" fn proc(h:HWND,m:u32,w:WPARAM,l:LPARAM)->LRESULT{DefWindowProcW(h,m,w,l)}
        let instance=GetModuleHandleW(None).unwrap().into();let pid=std::process::id();
        let paths=[normalized(&std::env::current_exe().unwrap().to_string_lossy())];
        let started=process_started(GetCurrentProcess()).unwrap();
        for (client,class) in [(Messenger::WeCom,w!("WeWorkWindow")),(Messenger::Feishu,w!("Chrome_WidgetWin_1")),(Messenger::DingTalk,w!("Qt51511QWindowIcon")),(Messenger::DingTalk,w!("StandardFrame_DingTalk"))]{
            RegisterClassW(&WNDCLASSW{lpfnWndProc:Some(proc),hInstance:instance,lpszClassName:class,..Default::default()});
            let create=||CreateWindowExW(WINDOW_EX_STYLE(0),class,w!("Daozai synthetic client"),WS_OVERLAPPEDWINDOW,-10000,-10000,400,500,None,None,instance,None).unwrap();
            let first=create();let original=Target{unread:Unread{source:client,pid,window:first.0 as isize},started};
            assert!(ready_with_paths(original,&paths),"唯一原窗口必须可识别");
            assert_eq!(flash_target(client,first,&paths),Some(original.unread));
            let helper=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!(""),WS_POPUP,-10000,-10000,400,500,None,None,instance,None).unwrap();
            let ambiguous=flash_target(client,helper,&paths);DestroyWindow(helper).unwrap();
            assert_eq!(ambiguous,Some(Unread::of(client)),"同进程但没有窗口归属关系的辅助窗口不能借用仅剩的主窗口");
            let owned=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!(""),WS_POPUP,-10000,-10000,400,500,first,None,instance,None).unwrap();
            let bound=flash_target(client,owned,&paths);DestroyWindow(owned).unwrap();
            assert_eq!(bound,Some(original.unread),"窗口明确属于原主窗口时可以识别");
            let second=create();
            assert!(!ready_with_paths(original,&paths),"同进程新增主窗口后不得继续按进程猜账号");
            assert_eq!(flash_target(client,first,&paths),Some(Unread::of(client)));
            DestroyWindow(first).unwrap();
            assert!(!ready_with_paths(original,&paths),"不能换成仍存活的其他窗口");
            if client==Messenger::Feishu || client==Messenger::DingTalk && class_name(second)!=DINGTALK_FRAME {
                assert_eq!(rebuild_target(original,&paths),None,"没有主窗口特征的独立窗口不能作为重建窗口");
            }
            assert_eq!(open(original,&paths,true),Err(OpenError::Unavailable));
            assert!(!IsWindowVisible(second).as_bool(),"不能打开其他窗口");
            DestroyWindow(second).unwrap();
        }
        // 不足主窗口尺寸的 StandardFrame 不应挡住同进程真正的 Qt 主窗口。
        let small=CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("StandardFrame_DingTalk"),w!(""),WINDOW_STYLE(0x060f0000),-10000,-10000,220,220,None,None,instance,None).unwrap();
        let qt=CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!(""),WINDOW_STYLE(0x860f0000),-10000,-10000,1200,800,None,None,instance,None).unwrap();
        let original=Target{unread:Unread{source:Messenger::DingTalk,pid,window:qt.0 as isize},started};
        let _=ShowWindow(small,SW_SHOWMINNOACTIVE);
        assert!(IsIconic(small).as_bool());
        assert!(!main_window(small,Messenger::DingTalk));
        assert_eq!(flash_target(Messenger::DingTalk,qt,&paths),Some(original.unread));
        assert!(ready_with_paths(original,&paths));
        let new_main=CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!(""),WINDOW_STYLE(0x860f0000),-10000,-10000,1200,800,None,None,instance,None).unwrap();
        assert_eq!(rebuild_target(original,&paths),None,"原窗口存活时不能重绑");
        DestroyWindow(qt).unwrap();
        let rebuilt=Target{unread:Unread{window:new_main.0 as isize,..original.unread},..original};
        assert_eq!(rebuild_target(original,&paths),Some(rebuilt),"原窗口销毁后允许同一进程中唯一可信的新主窗口");
        assert_eq!(rebuild_target(Target{started:started.wrapping_add(1),..original},&paths),None,"禁止重绑到复用 PID 的新进程");
        DestroyWindow(small).unwrap();DestroyWindow(new_main).unwrap();
        // 用户诊断包里钉钉 9.0.5 的窗口组合：一个 StandardFrame_DingTalk 主窗口和三个隐藏的 Qt 对话框。
        // 主窗口最小化到任务栏和隐藏到托盘时，都必须对应到原进程。
        for (state,frame_style) in [("最小化",0x260f0000u32),("隐藏到托盘",0x060f0000)]{
            let frame=CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("StandardFrame_DingTalk"),w!("Daozai synthetic client"),WINDOW_STYLE(frame_style),-10000,-10000,1200,800,None,None,instance,None).unwrap();
            let dialogs=[0;3].map(|_|CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!(""),WINDOW_STYLE(0x86cf0000),-10000,-10000,655,517,None,None,instance,None).unwrap());
            let original=Target{unread:Unread{source:Messenger::DingTalk,pid,window:frame.0 as isize},started};
            assert!(dialogs.iter().all(|dialog|!main_window(*dialog,Messenger::DingTalk)),"{state}：隐藏的 Qt 对话框不能算主窗口");
            assert!(ready_with_paths(original,&paths),"{state}：主窗口必须可识别");
            assert_eq!(flash_target(Messenger::DingTalk,frame,&paths),Some(original.unread),"{state}：闪动必须对应到原进程");
            DestroyWindow(frame).unwrap();
            assert!(dialogs.iter().all(|dialog|main_window(*dialog,Messenger::DingTalk)),"没有新版主窗口时仍按旧版 Qt 主窗口识别");
            assert_eq!(flash_target(Messenger::DingTalk,dialogs[0],&paths),Some(Unread::of(Messenger::DingTalk)),"旧版多个 Qt 主窗口仍不能猜账号");
            for dialog in dialogs{DestroyWindow(dialog).unwrap();}
        }
        // 钉钉 9.0.5 的 Qt 主窗口最小化时样式是 0xB60F0000。收进托盘后，此处模拟它清除可见和最小化标志。
        for (state,main_style) in [("显示",0x960f0000u32),("最小化",0xb60f0000),("关到托盘",0x860f0000)]{
            let main=CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!("Daozai synthetic client"),WINDOW_STYLE(main_style),-10000,-10000,1200,800,None,None,instance,None).unwrap();
            let dialogs=[0;3].map(|_|CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!(""),WINDOW_STYLE(0x86cf0000),-10000,-10000,655,517,None,None,instance,None).unwrap());
            let original=Target{unread:Unread{source:Messenger::DingTalk,pid,window:main.0 as isize},started};
            assert_eq!(flash_target(Messenger::DingTalk,main,&paths),Some(original.unread),"{state}：闪动必须对应到原主窗口");
            assert!(ready_with_paths(original,&paths),"{state}：点击时仍能确认原主窗口");
            assert_eq!(flash_target(Messenger::DingTalk,dialogs[0],&paths),Some(Unread::of(Messenger::DingTalk)),"{state}：无归属关系的对话框不能代替主窗口");
            let log=describe_process_windows(Messenger::DingTalk,pid);
            assert!(log.contains("Qt51511QWindowIcon") && log.contains("655×517") && !log.contains("Daozai"),"{state}：诊断只记类名和尺寸，不记标题：{log}");
            DestroyWindow(main).unwrap();for dialog in dialogs{DestroyWindow(dialog).unwrap();}
        }
        // 两个同样式的 Qt 主窗口无论显示还是隐藏，都不能凭闪动猜账号。
        for style in [0x16cf0000,0x860f0000]{
            let pair=[0;2].map(|_|CreateWindowExW(WINDOW_EX_STYLE(0x100),w!("Qt51511QWindowIcon"),w!(""),WINDOW_STYLE(style),-10000,-10000,1200,800,None,None,instance,None).unwrap());
            assert_eq!(flash_target(Messenger::DingTalk,pair[0],&paths),Some(Unread::of(Messenger::DingTalk)));
            for window in pair{DestroyWindow(window).unwrap();}
        }
        // 本机飞书 8.6 实测：主窗口标题「飞书」，「在独立窗口打开」的会话窗口与它同进程、同类名、同样式，标题是会话名。
        // 主窗口正常显示和关到托盘（隐藏）两种状态都要能对应到原进程，独立窗口闪动归到主窗口。
        for (state,main_style) in [("显示",0x16cf0000u32),("关到托盘",0x06cf0000)]{
            let frame=|title,style|CreateWindowExW(WINDOW_EX_STYLE(0x00200100),w!("Chrome_WidgetWin_1"),title,WINDOW_STYLE(style),-10000,-10000,1200,800,None,None,instance,None).unwrap();
            let main=frame(w!("飞书"),main_style);let chat=frame(w!("少爷"),0x16cf0000);
            let original=Target{unread:Unread{source:Messenger::Feishu,pid,window:main.0 as isize},started};
            assert!(!main_window(chat,Messenger::Feishu),"{state}：独立会话窗口不能算第二个主窗口");
            assert!(ready_with_paths(original,&paths),"{state}：有独立会话窗口时主窗口仍可识别");
            assert_eq!(flash_target(Messenger::Feishu,main,&paths),Some(original.unread),"{state}：主窗口闪动对应原进程");
            assert_eq!(flash_target(Messenger::Feishu,chat,&paths),Some(original.unread),"{state}：独立会话窗口闪动归到同进程主窗口");
            let lookalike=frame(w!("Feishu"),0x16cf0000);
            assert_eq!(flash_target(Messenger::Feishu,chat,&paths),Some(Unread::of(Messenger::Feishu)),"{state}：两个窗口都像主窗口时不猜");
            DestroyWindow(lookalike).unwrap();DestroyWindow(main).unwrap();
            assert!(main_window(chat,Messenger::Feishu),"{state}：找不到主窗口时沿用原规则");
            DestroyWindow(chat).unwrap();
        }
        // 同名小窗与主窗口最小化后的系统矩形相似，应以恢复尺寸区分。
        let create=|title,width,height|CreateWindowExW(WINDOW_EX_STYLE(0),w!("Chrome_WidgetWin_1"),title,WS_OVERLAPPEDWINDOW,-10000,-10000,width,height,None,None,instance,None).unwrap();
        let main=create(w!("飞书"),998,726);
        let small=create(w!("Feishu"),220,220);
        let chat=create(w!("独立会话"),400,500);
        for window in [main,small]{
            let _=ShowWindow(window,SW_SHOWMINNOACTIVE);
            assert!(IsIconic(window).as_bool(),"测试窗口必须进入最小化状态");
        }
        let original=Target{unread:Unread{source:Messenger::Feishu,pid,window:main.0 as isize},started};
        assert!(!main_window(small,Messenger::Feishu),"真正的小窗最小化后仍应被排除");
        assert!(ready_with_paths(original,&paths),"正常的主窗口最小化后仍可识别");
        assert_eq!(flash_target(Messenger::Feishu,main,&paths),Some(original.unread));
        assert_eq!(flash_target(Messenger::Feishu,chat,&paths),Some(original.unread),"同名小窗不影响独立会话的归属");
        assert_eq!(flash_target(Messenger::Feishu,small,&paths),Some(Unread::of(Messenger::Feishu)),"同名小窗闪动不应借用主窗口");
        for window in [chat,small,main]{DestroyWindow(window).unwrap();}
    }}
    #[test]
    fn hidden_helpers_are_never_shown(){unsafe{
        let paths=[normalized(&std::env::current_exe().unwrap().to_string_lossy())];
        let window=CreateWindowExW(WS_EX_TOOLWINDOW,w!("STATIC"),w!(""),WS_POPUP,-10000,-10000,400,500,None,None,None,None).unwrap();
        for source in Messenger::OFFICE{
            assert_eq!(open(target(source,std::process::id(),window.0 as isize),&paths,true),Err(OpenError::Unavailable));
            assert!(!IsWindowVisible(window).as_bool());
        }
        DestroyWindow(window).unwrap();
    }}
}
