'use strict';
const assert=require('node:assert/strict');
const {Client}=require('pg');
const base={host:process.env.DEP_HOST||'127.0.0.1',port:Number(process.env.DEP_PORT||5543),ssl:false};
async function asRole(user,password,database,check){const c=new Client({...base,user,password,database});await c.connect();try{await check(c);}finally{await c.end();}}
async function refused(c,sql,code){await assert.rejects(()=>c.query(sql),error=>error.code===code,'Expected '+code);}
async function main(){
  await asRole('demo_app','demo-local','operational',async c=>{
    const r=await c.query('SELECT full_name, national_id FROM patients WHERE national_id = $1',['123456782']);
    assert.deepEqual(r.rows,[{full_name:'Dana Cohen',national_id:null}]);
    await refused(c,"SELECT full_name FROM patients WHERE national_id LIKE '123%'",'ZD101');
    await refused(c,'SELECT p.full_name, r.diagnosis FROM patients p JOIN records r ON r.patient_national_id = p.national_id','ZD107');
    await refused(c,'SELECT * FROM patient_identifier(1)','42501');
  });
  await asRole('demo_analytics','demo-local','analytics',c=>refused(c,"SELECT full_name FROM patients WHERE national_id = '123456782'",'ZD102'));
  await asRole('export_job','export-local','operational',async c=>{
    assert.deepEqual((await c.query('SELECT * FROM patient_identifier(1)')).rows,[{national_id:'123456782'}]);
    await refused(c,'SELECT * FROM patient_identifier(1) ORDER BY national_id','ZD104');
  });
  console.log('PASS: lookup output, partial match, protected join, role grants, analytics, disclosure and terminal rule.');
}
main().catch(error=>{console.error(error.code||error.message);process.exitCode=1;});
