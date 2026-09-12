// Read-only prototype: decrypts/inflates BTD6 Profile.Save or MapSaveData and dumps JSON.
// Usage: node tools/inspect_save.js <outDir> <file>...   (see SPEC.md section 2.5)
const fs=require('fs'),crypto=require('crypto'),zlib=require('zlib'),path=require('path');
function parse(buf){
  const version=buf.readUInt32LE(0), inner=buf.readUInt32LE(4), counter=buf.readUInt32LE(8);
  // 44 fixed bytes, then 7-bit-length-prefixed string
  let off=44; const slen=buf[off]; off+=1; const str=buf.slice(off,off+slen).toString('latin1'); off+=slen;
  let info={version,inner,counter,str,headerEnd:off};
  let payload;
  if(buf[off]===0x78){ // raw zlib, unencrypted
    info.encrypted=false; payload=buf.slice(off);
  } else {
    const pwIndex=Number(buf.readBigUInt64LE(off)); off+=8;
    const salt=buf.slice(off,off+24); off+=24;
    const enc=buf.slice(off);
    const kiv=crypto.pbkdf2Sync('11',salt,10,32,'sha1');
    const d=crypto.createDecipheriv('aes-128-cbc',kiv.slice(16,32),kiv.slice(0,16));
    payload=Buffer.concat([d.update(enc),d.final()]);
    info.encrypted=true; info.pwIndex=pwIndex; info.encLen=enc.length;
  }
  const json=zlib.inflateSync(payload).toString('utf8').replace(/^\uFEFF/,'');
  return {info,json};
}
const out=process.argv[2]; fs.mkdirSync(out,{recursive:true});
for(const f of process.argv.slice(3)){
  try{
    const {info,json}=parse(fs.readFileSync(f));
    const obj=JSON.parse(json);
    const name=path.basename(path.dirname(f)).slice(0,8)+'_'+path.basename(f)+'.json';
    fs.writeFileSync(path.join(out,name),JSON.stringify(obj,null,1));
    console.log('=== '+f); console.log(JSON.stringify(info)); console.log('json bytes:',json.length,' top-level type:',Array.isArray(obj)?'array':typeof obj);
    if(obj&&typeof obj==='object'&&!Array.isArray(obj)){
      const keys=Object.keys(obj); console.log('keys('+keys.length+'):',keys.join(', ').slice(0,3000));
    }
  }catch(e){console.log('=== '+f+' FAILED: '+e.message);}
}
