// Exercise the shipped browser preflight against the same caller-supplied engine bounds.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const {readFileSync}=require('node:fs');
const vm=require('node:vm');
const path=require('node:path');
const source=readFileSync(path.join(__dirname,'../crates/app/src/native_sources.rs'),'utf8').split('inline_js = r#"')[1].split('"#)]')[0].replaceAll('export function','function');
function picker(files){
 const input={files,click(){queueMicrotask(()=>this.onchange());}};
 const context={document:{createElement:()=>input},TextDecoder,Promise,Object};
 vm.runInNewContext(source,context);
 return context;
}
function file(path,text='{}',size=Buffer.byteLength(text)){
 return {webkitRelativePath:`library/${path}`,size,arrayBuffer:async()=>new TextEncoder().encode(text).buffer};
}
test('folder accepts thousands of documents with engine-provided limits',async()=>{
 const files=[file('library.json'),...Array.from({length:3200},(_,i)=>file(`parts/part-${i}.json`))];
 const result=await picker(files).pickLibraryFolder(8192,1048576,33554432);
 assert.equal(Object.keys(result.documents).length,3201);
 assert.equal(result.problems.length,0);
});
test('folder and bundle preflight retain caller bounds',async()=>{
 await assert.rejects(picker([file('library.json'),file('parts/a.json')]).pickLibraryFolder(1,1048576,33554432),/1 files/);
 await assert.rejects(picker([file('library.json','{}',6),file('parts/a.json','{}',6)]).pickLibraryFolder(8192,1048576,10),/source exceeds/);
 await assert.rejects(picker([file('library.json','{}',33554432*2+1)]).pickLibraryBundle(8192,1048576,33554432),/64 MiB/);
 const result=await picker([file('library.json'),file('parts/a.json','{}',1048577)]).pickLibraryFolder(8192,1048576,33554432);
 assert.equal(result.problems.length,1);
 assert.equal(Object.keys(result.documents).length,1);
});
