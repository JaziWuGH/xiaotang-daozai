//! QQ 的安装位置、主窗口和托盘规则。
use super::{Messenger,OpenError,Target,Unread};
use super::windows::*;
use windows::{core::{w,PCWSTR},Win32::{Foundation::{HWND,LPARAM,WPARAM},System::{Registry::{HKEY_CLASSES_ROOT,HKEY_CURRENT_USER},Threading::{OpenProcess,PROCESS_QUERY_LIMITED_INFORMATION}},UI::WindowsAndMessaging::*}};
use std::path::PathBuf;

pub(crate) const ADAPTER:NativeAdapter=NativeAdapter{paths,main_frame,accounts:Some(accounts),ready,strict_window:true,open,taskbar:None};
fn main_frame(class:&str,style:u32,extended:u32)->bool{matches!(class,"Chrome_WidgetWin_1"|"TXGuiFoundation") && frame_style(class,style,extended)}
fn open(unread:Unread,paths:&[String],_:bool)->Result<(),OpenError>{unsafe{
    let window=HWND(unread.window as *mut _);
    if unread.window==0 || !IsWindow(window).as_bool() || window_pid(window)!=unread.pid{return Err(OpenError::Unavailable);}
    let target=if usable_main_frame(window,unread.source) && matches(window,paths){Some(window)}else{visible_main_window_of(unread.pid,unread.source,paths)};
    if let Some(target)=target{
        if IsIconic(target).as_bool(){let _=ShowWindowAsync(target,SW_RESTORE);}
        if SetForegroundWindow(target).as_bool(){return Ok(());}
    }
    if class_name(window)=="Electron_NotifyIconHostWindow" && matches(window,paths){
        let icons=host_icons(&[(unread.pid,window)],unread.pid);
        if let [(_,id)]=icons.as_slice(){
            for event in [WM_LBUTTONDOWN,WM_LBUTTONUP,WM_LBUTTONDBLCLK,WM_LBUTTONUP]{
                PostMessageW(window,WM_APP+1,WPARAM(*id as usize),LPARAM(event as isize)).map_err(|_|OpenError::Failed("QQ 托盘暂时无法响应".into()))?;
            }
            return Ok(());
        }
    }
    Err(OpenError::Unavailable)
}}

fn paths()->Vec<String>{
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let mut candidates=Vec::new();
    for root in [HKEY_CURRENT_USER,HKEY_LOCAL_MACHINE]{
        for key in [w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\QQ"),w!("Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\QQ")]{
            if let Some(icon)=registry_string(root,key,w!("DisplayIcon")){
                let exe=icon.rsplit_once(',').filter(|(_,index)|index.trim().parse::<i32>().is_ok()).map_or(icon.as_str(),|(path,_)|path);
                candidates.push(PathBuf::from(exe.trim().trim_matches('"')));
            }
        }
    }
    // QQ NT 的协议处理器在版本子目录中，主程序在安装根目录。
    if let Some(command)=registry_string(HKEY_CLASSES_ROOT,w!("tencent\\shell\\open\\command"),PCWSTR::null()){
        let helper=command.trim().trim_start_matches('"').split('"').next().unwrap_or("");
        let helper=std::path::Path::new(helper);
        if helper.is_absolute() && helper.file_name().is_some_and(|n|n.eq_ignore_ascii_case("timwp.exe")){
            if let Some(versions)=helper.ancestors().find(|p|p.file_name().is_some_and(|n|n.eq_ignore_ascii_case("versions"))){
                if let Some(root)=versions.parent(){candidates.push(root.join("QQ.exe"));}
            }
        }
    }
    let mut paths:Vec<String>=candidates.into_iter().filter(|p|p.is_absolute() && p.file_name().is_some_and(|n|n.eq_ignore_ascii_case("QQ.exe")) && p.is_file())
        .filter_map(|p|p.canonicalize().ok()).map(|p|normalized(&p.to_string_lossy())).collect();
    paths.push("*qq.exe".into());
    paths
}

fn ready(target:Target)->bool{unsafe{
    let window=HWND(target.unread.window as *mut _);
    if !IsWindow(window).as_bool() || window_pid(window)!=target.unread.pid{return false;}
    if class_name(window)!="Electron_NotifyIconHostWindow"{return true;}
    // 同一进程出现多个宿主或图标时，不能按进程号确认正在查看哪个账号。
    struct Hosts{pid:u32,count:usize}
    unsafe extern "system" fn count(hwnd:HWND,data:LPARAM)->windows::Win32::Foundation::BOOL{
        let hosts=&mut *(data.0 as *mut Hosts);
        if window_pid(hwnd)==hosts.pid && class_name(hwnd)=="Electron_NotifyIconHostWindow"{hosts.count+=1;}
        true.into()
    }
    let mut hosts=Hosts{pid:target.unread.pid,count:0};
    if EnumWindows(Some(count),LPARAM((&mut hosts as *mut Hosts)as isize)).is_err(){return false;}
    hosts.count==1 && host_icons(&[(target.unread.pid,window)],target.unread.pid).len()==1
}}

pub(crate) fn accounts(paths:&[String])->Option<Vec<Target>>{unsafe{
    use windows::Win32::Foundation::BOOL;
    struct Search<'a>{paths:&'a [String],hosts:Vec<HWND>,mains:Vec<HWND>}
    unsafe extern "system" fn visit(hwnd:HWND,data:LPARAM)->BOOL{
        let search=&mut *(data.0 as *mut Search);
        let class=class_name(hwnd);
        let tray=class=="Electron_NotifyIconHostWindow";
        let main=client_main_frame(Messenger::Qq,&class,GetWindowLongW(hwnd,GWL_STYLE)as u32,GetWindowLongW(hwnd,GWL_EXSTYLE)as u32)
            && GetWindow(hwnd,GW_OWNER).is_err();
        if (tray || main) && matches(hwnd,search.paths){if tray{search.hosts.push(hwnd);}else{search.mains.push(hwnd);}}
        true.into()
    }
    let mut search=Search{paths,hosts:Vec::new(),mains:Vec::new()};
    EnumWindows(Some(visit),LPARAM((&mut search as *mut Search)as isize)).ok()?;
    if search.hosts.is_empty() && search.mains.is_empty(){return Some(Vec::new());}
    for &host in &search.hosts{if host_icons(&[(window_pid(host),host)],window_pid(host)).len()!=1{return None;}}
    let owners:Vec<_>=search.hosts.iter().map(|&hwnd|window_pid(hwnd)).collect();
    let parents=parent_processes();let mut windows=search.hosts;
    for main in search.mains{
        let pid=window_pid(main);
        if account_tray_pid(pid,&owners,&parents).is_none() && !windows.iter().any(|&hwnd|window_pid(hwnd)==pid){windows.push(main);}
    }
    windows.into_iter().map(|hwnd|{
        let pid=window_pid(hwnd);
        let process=windows::core::Owned::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,pid).ok()?);
        Some(Target{unread:Unread{source:Messenger::Qq,pid,window:hwnd.0 as isize},started:process_started(*process)?})
    }).collect()
}}
