//! Temporary evidence generator; classification is authored only through shared commands.
use pcbcraft_engine::{Session, persistence::{decode_packed,encode_packed}};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
fn apply(s:&mut Session,command:&str,mut params:Value)->Result<(),Box<dyn std::error::Error>> {
 params["expected_revision"]=json!(s.revision());s.apply(command,&params)?;Ok(())
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let args:Vec<_>=std::env::args().collect();
 let mut artifact:Value=serde_json::from_slice(&std::fs::read(&args[1])?)?;
 let bytes:Vec<u8>=serde_json::from_value(artifact["project"].clone())?;
 let mut author=Session::default();author.open_snapshot(decode_packed(&bytes)?.project);
 apply(&mut author,"net_classes.create",json!({"name":"Power","rules":{"clearance_nm":700000,"min_track_width_nm":400000,"preferred_track_width_nm":500000}}))?;
 let class=author.project().ok_or("missing project")?.net_classes().definitions[0].id;
 let targets=author.project().ok_or("missing project")?.inspect_net_registry()?;
 let anchor=targets["nets"][0]["anchor"].clone();
 apply(&mut author,"nets.set_classes",json!({"target":{"kind":"derived","anchor":anchor},"classes":[class]}))?;
 let bytes=encode_packed(author.snapshot().ok_or("missing source")?)?;
 artifact["project_sha256"]=json!(format!("{:x}",Sha256::digest(&bytes)));
 artifact["project"]=json!(bytes);
 artifact["recipe"]=json!("reference-v1-class-assigned");
 artifact["generator_revision"]=json!("issue302-command-assignment");
 let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(&args[2])?;
 std::io::Write::write_all(&mut output,&serde_json::to_vec(&artifact)?)?;
 Ok(())
}
