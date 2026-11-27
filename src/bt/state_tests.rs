use super::*;

fn endpoint(id:&str,address:&str,container:Option<&str>)->Endpoint {
    Endpoint {id:id.into(),address:normalize_address(address),container:container.map(str::to_owned),name:"同名耳机".into(),kind:DeviceKind::Earbuds,ble:false}
}
fn sample(key:&str,session:u64,source:BatterySource,percent:u8,at:Instant)->Sample {
    Sample {key:key.into(),session,reading:BatteryReading::new(percent,source,at,None)}
}

#[test]
fn same_address_and_container_merge_but_names_never_do(){
    let now=Instant::now();let mut state=State::default();
    state.discover(vec![endpoint("classic","AA:BB:CC:DD:EE:01",Some("group")),endpoint("ble","AABBCCDDEE01",None),endpoint("hid","AABBCCDDEE02",Some("group")),endpoint("other","AABBCCDDEE03",None)],now);
    assert_eq!(state.devices(now).len(),2);
    assert_eq!(state.targets().len(),4);
    let key=state.targets()[0].key.clone();
    state.discover(vec![endpoint("ble","AABBCCDDEE01",None),endpoint("other","AABBCCDDEE03",None)],now);
    assert!(state.devices(now).iter().any(|d|d.id==key&&d.connected));
}

#[test]
fn no_identity_and_partial_mac_must_not_collapse_devices(){
    let now=Instant::now();let mut state=State::default();
    state.discover(vec![endpoint("first","",None),endpoint("second","",None)],now);
    assert_eq!(state.devices(now).len(),2);
    assert_eq!(normalize_address("AA:BB:CC:DD:EE:FF"),Some("AABBCCDDEEFF".into()));
    assert_eq!(normalize_address(""),None);
    assert_eq!(normalize_address("000000000000"),None);
    assert_eq!(pnp_address("BTHENUM\\DEV_AABBCCDDEEFF\\7&123"),Some("AABBCCDDEEFF".into()));
    assert_eq!(pnp_address("BTHENUM\\DEV_AABBCCDDEEFF0\\7&123"),None);
    assert_eq!(pnp_address("USB\\VID_AABBCCDDEEFF"),None);
}

#[test]
fn fresh_gatt_wins_cached_pnp_and_zero_is_valid(){
    let now=Instant::now();let mut state=State::default();state.discover(vec![endpoint("ble","AABBCCDDEE01",None)],now);
    let t=state.targets().remove(0);
    state.ingest(sample(&t.key,t.session,BatterySource::Gatt,0,now));
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,90,now+Duration::from_secs(1)));
    state.ingest(sample(&t.key,t.session,BatterySource::Gatt,255,now+Duration::from_secs(2)));
    assert_eq!(state.devices(now+Duration::from_secs(2))[0].battery,Some(0));
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,90,now+Duration::from_secs(182)));
    let d=state.devices(now+Duration::from_secs(182)).remove(0);
    assert!(!d.stale_at(now+Duration::from_secs(182)));
    assert_eq!(d.battery,Some(90));assert_eq!(d.reading.unwrap().source,BatterySource::Windows);
}

#[test]
fn late_samples_cannot_overwrite_newer_samples_or_reconnected_devices(){
    let now=Instant::now();let mut state=State::default();let endpoints=vec![endpoint("ble","AABBCCDDEE01",None)];
    state.discover(endpoints.clone(),now);let old=state.targets().remove(0);
    state.ingest(sample(&old.key,old.session,BatterySource::Gatt,70,now+Duration::from_secs(2)));
    state.ingest(sample(&old.key,old.session,BatterySource::Gatt,80,now));
    assert_eq!(state.devices(now+Duration::from_secs(3))[0].battery,Some(70));
    state.discover(vec![],now+Duration::from_secs(4));
    assert!(!state.devices(now+Duration::from_secs(4))[0].connected);
    state.discover(endpoints,now+Duration::from_secs(5));
    state.ingest(sample(&old.key,old.session,BatterySource::Gatt,99,now+Duration::from_secs(6)));
    assert_eq!(state.devices(now+Duration::from_secs(6))[0].battery,None);
}

#[test]
fn errors_age_readings_and_disconnected_history_expires(){
    let now=Instant::now();let mut state=State::default();state.discover(vec![endpoint("ble","AABBCCDDEE01",None)],now);
    let t=state.targets().remove(0);state.ingest(sample(&t.key,t.session,BatterySource::Windows,44,now));
    let d=state.devices(now+Duration::from_secs(181)).remove(0);
    assert_eq!(d.battery,Some(44));assert!(d.stale_at(now+Duration::from_secs(181)));
    state.discover(vec![],now+Duration::from_secs(182));
    state.discover(vec![],now+Duration::from_secs(900));
    assert!(state.devices(now+Duration::from_secs(900)).is_empty());
}

#[test]
fn windows_service_nodes_only_match_the_remote_address_field() {
    let service="{0000180F-0000-1000-8000-00805F9B34FB}";
    for instance in [
        format!(r"BTHLEDEVICE\{service}_DEV_VID&0204E8_PID&7021_REV&0001_AABBCCDDEE01\8&3B77208C&0&002A"),
        format!(r"BTHLEDEVICE\{service}_AABBCCDDEE01\8&3B77208C&0&002A"),
        format!(r"BTHENUM\{service}_VID&0001000A_PID&FFFF\7&331D6FF8&0&AABBCCDDEE01_C00000000"),
    ] {
        assert_eq!(pnp_address(&instance).as_deref(),Some("AABBCCDDEE01"));
    }
    for instance in [
        format!(r"BTHLEDEVICE\{service}\8&3B77208C&0&002A"),
        format!(r"BTHENUM\{service}\7&331D6FF8&0&000000000000_C00000000"),
        format!(r"USB\{service}_AABBCCDDEE01\1"),
    ] { assert_eq!(pnp_address(&instance),None); }
}

#[test]
fn windows_poll_refreshes_same_value_and_rename_preserves_identity() {
    let now=Instant::now();
    let mut state=State::default();
    let mut ep=endpoint("classic","AABBCCDDEE01",None);
    state.discover(vec![ep.clone()],now);
    let t=state.targets().remove(0);
    let later=now+FRESH_FOR+Duration::from_secs(1);
    for at in [now,later] {state.ingest(sample(&t.key,t.session,BatterySource::Windows,20,at));}
    let d=state.devices(later).remove(0);
    assert!(!d.stale_at(later));
    assert_eq!(d.reading.as_ref().unwrap().value_since,now);
    assert_eq!(d.reading.as_ref().unwrap().observed,later);
    assert!(!d.stale_at(later+FRESH_FOR));
    assert!(d.stale_at(later+FRESH_FOR+Duration::from_secs(1)));
    ep.name="新的耳机名称".into();
    state.discover(vec![ep],later);
    let d=state.devices(later).remove(0);
    assert_eq!((d.id,d.name,d.battery),(t.key.clone(),"新的耳机名称".into(),Some(20)));
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,19,later+Duration::from_secs(2)));
    assert_eq!(state.devices(later+Duration::from_secs(2))[0].reading.as_ref().unwrap().value_since,later+Duration::from_secs(2));
}

#[test]
fn windows_cached_value_after_reconnect_needs_a_poll_and_keeps_age() {
    let now=Instant::now();
    let mut state=State::default();
    let ep=endpoint("ble","AABBCCDDEE01",None);
    state.discover(vec![ep.clone()],now);
    let t=state.targets().remove(0);
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,20,now));
    let later=now+FRESH_FOR+Duration::from_secs(1);
    state.discover(vec![],later);
    state.discover(vec![ep],later);
    assert!(state.devices(later)[0].reading.is_none());
    let t=state.targets().remove(0);
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,20,later));
    assert!(!state.devices(later)[0].stale_at(later));
    assert_eq!(state.devices(later)[0].reading.as_ref().unwrap().value_since,now);
    state.ingest(sample(&t.key,t.session,BatterySource::Windows,10,later-Duration::from_secs(1)));
    assert_eq!(state.devices(later)[0].battery,Some(20));
}
