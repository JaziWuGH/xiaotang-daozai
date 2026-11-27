use crate::{bar_layout::*,paint::{self,Card,H},resident_date::ResidentDay,theme::{self,W}};
use eframe::egui::{self,Color32,RichText,Rect,Pos2,Vec2};

pub(crate) fn preview_layout(order:&[usize],quotas:usize,performance_width:f32)->Layout {
    let mut layout=Layout::new(true,quotas.max(1),true,true,false,true,true);
    layout.add_performance(performance_width);layout.reorder(order);layout
}

impl crate::app::App {
    pub(crate) fn resident_order_settings(&mut self,ui:&mut egui::Ui){
        let default=if self.settings.show_battery {DEFAULT_ORDER.to_vec()}else{vec![QUOTA,PET,CLOCK,WEATHER,BATTERY,MUSIC,FOCUS,PERFORMANCE]};
        let mut order=if valid_order(&self.settings.resident_order){self.settings.resident_order.clone()}else{default.clone()};
        ui.horizontal(|ui|{
            ui.label(RichText::new("拖动岛上的图标").color(ui.visuals().text_color()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                if ui.small_button("恢复默认").clicked(){self.settings.resident_order.clear();order=default;}
            });
        });
        ui.add_space(12.0);
        let indices=self.quota_bar_indices();let count=indices.len();
        let weather_snapshot=self.weather_pill().cloned();
        let performance_enabled=self.module_on(crate::app::PERFORMANCE_PAGE) && self.settings.performance_bar.width(self.settings.performance_metrics)>0.0;
        let performance_width=if performance_enabled {self.settings.performance_bar.width(self.settings.performance_metrics)}else{20.0};
        let mut layout=preview_layout(&order,count,performance_width);
        for (position,extra) in self.quota_balance_layout(ui.ctx()){layout.balance_tail(position,extra);}
        let now=chrono::Local::now();
        let clock_text=self.settings.clock_text(&now.format("%H:%M:%S").to_string());
        let clock=self.settings.show_clock.then_some(clock_text.as_str());
        let day=ResidentDay::new(self.settings.resident_date,now.date_naive());
        if let Some(day)=&day {
            layout.set_clock_width(day.group_width(ui.ctx(),clock));
        }else{
            if self.settings.clock_period!=0 { layout.widen_clock(40.0); }
            if self.settings.clock_seconds { layout.widen_clock(24.0); }
        }
        if let Some(extra)=self.weather_pill_extra(ui.ctx()){layout.widen(WEATHER,extra);}
        let (stage,_)=ui.allocate_exact_size(Vec2::new(ui.available_width(),100.0),egui::Sense::hover());
        ui.painter().rect_filled(stage,10.0,Color32::from_rgb(19,25,36));
        let scale=((stage.width()-28.0)/layout.width).min(1.0);
        let origin=Pos2::new(stage.center().x-layout.width*scale/2.0,stage.top());
        // 预览跟随当前的皮肤与形状。
        let pill=self.settings.island_shape==crate::model::IslandShape::Pill;
        let origin=if pill{origin+Vec2::new(0.0,4.0)}else{origin};
        let body=Rect::from_min_size(origin,Vec2::new(layout.width*scale,30.0*scale));
        let fill=self.preview_fill();
        if pill {ui.painter().rect_filled(body,15.0*scale,fill);}
        else {crate::shape::fill(ui.painter(),body,8.0*scale,10.0*scale,12.0*scale,fill,1.0/ui.ctx().pixels_per_point());}
        if self.effective_skin().is_glass() {
            let rim=egui::Stroke::new(1.0_f32,Color32::from_white_alpha(60));
            if pill {ui.painter().rect_stroke(body,15.0*scale,rim,egui::StrokeKind::Inside);}
            else {
                let edge=Rect::from_min_max(body.min+Vec2::new(0.5,0.0),body.max-Vec2::new(0.5,0.5));
                ui.painter().add(egui::Shape::line(crate::shape::outline(edge,8.0*scale,10.0*scale,12.0*scale-0.5),rim));
            }
        }
        let content_from=crate::skin_paint::shape_count(ui.ctx(),ui.layer_id());
        let mut card=Card::new(ui,origin,stage);card.s=scale;
        let cx=layout.width/2.0;
        let pointer=ui.ctx().pointer_interact_pos();
        let dragged=egui::DragAndDrop::payload::<usize>(ui.ctx());
        let mut movement=None;
        let label=|slot|match slot{
            CLOCK if day.is_some()=>if clock.is_some(){"时间和日期"}else{"日期"},
            CLOCK=>"时间",BATTERY=>"电量",WEATHER=>"天气",QUOTA=>"AI 额度",MUSIC=>"音乐",FOCUS=>"专注",PERFORMANCE=>"系统性能",_=>"吉祥物"};
        for (index,&slot) in order.iter().enumerate(){
            let x=cx+layout.x[slot].unwrap();
            let width=match slot{CLOCK=>layout.widths[CLOCK],BATTERY=>20.0,WEATHER=>layout.widths[WEATHER],PET=>32.0,MUSIC=>14.0,FOCUS=>crate::focus_bar::WIDTH,PERFORMANCE=>layout.widths[PERFORMANCE],_=>{
                let left=layout.quota_slots().filter_map(|i|layout.x[i].map(|x|x-layout.widths[i]/2.0)).fold(f32::INFINITY,f32::min);
                let right=layout.quota_slots().filter_map(|i|layout.x[i].map(|x|x+layout.widths[i]/2.0)).fold(f32::NEG_INFINITY,f32::max);
                right-left
            }};
            let left=if slot==QUOTA{cx+layout.quota_slots().filter_map(|i|layout.x[i].map(|x|x-layout.widths[i]/2.0)).fold(f32::INFINITY,f32::min)}else{x-width/2.0};
            let rect=card.rect(left-5.0,1.0,width+10.0,28.0);
            let response=ui.interact(rect,ui.id().with(("resident-icon",slot)),egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab);
            response.dnd_set_drag_payload(slot);
            if let (Some(from),Some(point))=(dragged.as_deref(),pointer){
                if *from!=slot && rect.contains(point) && ui.input(|i|i.pointer.primary_down()){movement=Some((*from,index));}
            }
            let is_dragged=dragged.as_deref()==Some(&slot);
            let dx=if is_dragged{pointer.map(|p|((p.x-origin.x)/scale-(left+width/2.0)).clamp(-left,layout.width-left-width)).unwrap_or(0.0)}else{0.0};
            if is_dragged {ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);}
            else if response.hovered(){ui.painter().rect_filled(rect,6.0,Color32::from_white_alpha(18));}
            let active=match slot{CLOCK=>clock.is_some()||day.is_some(),BATTERY=>self.settings.show_battery&&self.resident_power.0.is_some(),WEATHER=>weather_snapshot.is_some(),PET=>self.settings.pet,QUOTA=>count>0,MUSIC=>self.media.is_music(),FOCUS=>self.pomo.running,PERFORMANCE=>self.performance_on_bar(),_=>true};
            let c=card.shifted(dx,0.0).faded(if active||is_dragged{1.0}else{0.38}).ghost();
            match slot{
                PERFORMANCE if performance_enabled=>self.paint_performance_bar(&c,x),
                PERFORMANCE=>{c.arc(x,15.0,7.0,2.25,7.18,2.0,theme::GRAY);c.seg((x,15.0),(x+3.0,11.5),1.8,theme::WHITE);},
                BATTERY=>paint::battery(&c,x,15.0,self.resident_power.0,self.resident_power.1,1.0),
                CLOCK=>{
                    if let Some(day)=&day {
                        day.draw_group(&c,x,clock);
                    }else{
                        paint::clock(&c,x-paint::clock_width(&c,&clock_text,theme::size::PILL_CLOCK)/2.0,21.0,&clock_text,theme::size::PILL_CLOCK);
                    }
                },
                WEATHER=>if let Some(snapshot)=weather_snapshot.as_ref(){self.paint_pill_weather(&c,x,&layout,snapshot);}else{crate::weather_ui::icon(&c,x,15.0,16.0,crate::weather::Kind::Partly);},
                PET=>self.pet.draw(&c,x,self.t,self.skin_palette()),
                MUSIC=>paint::playback_bars(&c,x,15.0,self.t,self.media.playing,theme::WHITE),
                FOCUS=>crate::focus_bar::draw(&c,x,15.0,self.t,&self.pomo,self.phase_secs(self.pomo.phase),false),
                QUOTA if count>0=>self.draw_quota_bar(&c,cx,&layout),
                _=>{c.ring(x,15.0,10.0,2.0,theme::GRAY);c.text(x,18.0,"AI",7.0,W::Bold,theme::WHITE,H::Center);},
            }
            if is_dragged {ui.painter().line_segment([Pos2::new(rect.center().x,body.bottom()+6.0),Pos2::new(rect.center().x,body.bottom()+12.0)],egui::Stroke::new(2.0,theme::BLUE));}
            response.on_hover_text(label(slot));
        }
        if let Some(palette)=self.skin_palette() {
            let end=crate::skin_paint::shape_count(ui.ctx(),ui.layer_id());
            crate::skin_paint::recolor_layer(ui.ctx(),ui.layer_id(),&[(content_from,end)],&palette);
        }
        if let Some((slot,index))=movement{
            if let Some(from)=order.iter().position(|value|*value==slot){order.remove(from);order.insert(index,slot);self.settings.resident_order=order;}
        }
        ui.painter().text(Pos2::new(stage.center().x,stage.bottom()-22.0),egui::Align2::CENTER_CENTER,"按住图标左右拖动",theme::font(11.0,W::Regular),Color32::from_gray(160));
        ui.add_space(8.0);
        ui.label(RichText::new("灰色图标为暂未显示的组件；位置会保留。").size(11.0).color(ui.visuals().weak_text_color()));
        #[cfg(feature="sdk-js")]
        self.plugins.pinned_order_settings(ui);
    }
}
