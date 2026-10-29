/* The homepage displays build-verified simulator results. Execute them in Playground. */
(function () {
  'use strict';
  const demo = document.getElementById('home-demo');
  if (!demo) return;
  const tabs = [...demo.querySelectorAll('[role=tab]')];
  const query = demo.querySelector('#demo-query');
  const title = demo.querySelector('#demo-task');
  const explanation = demo.querySelector('#demo-explanation');
  const next = demo.querySelector('#demo-next');
  const link = demo.querySelector('#demo-run');
  const script = [...document.scripts].find(s=>s.src && new URL(s.src).pathname.endsWith('/assets/js/home-demo.js'));
  let examples;
  function result(target, data) {
    target.replaceChildren();
    if (data.error) {
      const code=document.createElement('strong');code.className='demo-error-code';code.textContent=data.error.sqlstate+' · QUERY REFUSED';
      const message=document.createElement('p');message.className='demo-error-message';message.textContent='Protected join has no one-person anchor.';
      target.append(code,message);return;
    }
    const table=document.createElement('table');
    const head=document.createElement('thead');const header=document.createElement('tr');
    data.columns.forEach(name=>{const cell=document.createElement('th');cell.textContent=name;header.append(cell);});head.append(header);
    const body=document.createElement('tbody');
    data.rows.slice(0,2).forEach(row=>{const tr=document.createElement('tr');row.forEach(value=>{const td=document.createElement('td');td.textContent=value===null?'NULL':String(value);if(value===null)td.className='demo-null';tr.append(td);});body.append(tr);});
    table.append(head,body);target.append(table);
    const count=document.createElement('p');count.className='demo-row-count';count.textContent=data.rows.length+' row'+(data.rows.length===1?'':'s')+(data.rows.length>2?' · showing 2':'');target.append(count);
  }
  function select(id, focus=false) {
    if (!examples) return;
    const example=examples.find(example=>example.id===id);if(!example)return;
    tabs.forEach(tab=>{const active=tab.dataset.example===id;tab.setAttribute('aria-selected',String(active));tab.tabIndex=active?0:-1;if(active&&focus)tab.focus();});
    demo.querySelector('#demo-panel').setAttribute('aria-labelledby','demo-tab-'+id);
    title.textContent=example.title;
    query.textContent=example.sql;
    if(window.HoopeoCode)HoopeoCode.highlight(query);
    result(demo.querySelector('#demo-regular'),example.regular);result(demo.querySelector('#demo-dep'),example.dep);
    explanation.textContent=example.explanation;next.textContent=example.next;
    const target=new URL('../../playground/?example='+encodeURIComponent(id),script.src);
    link.href=target.pathname+target.search;
    link.textContent='Run this exact query in the playground →';
  }
  tabs.forEach((tab,i)=>{
    tab.addEventListener('click',()=>select(tab.dataset.example));
    tab.addEventListener('keydown',event=>{let n;if(event.key==='ArrowRight')n=(i+1)%tabs.length;else if(event.key==='ArrowLeft')n=(i+tabs.length-1)%tabs.length;else if(event.key==='Home')n=0;else if(event.key==='End')n=tabs.length-1;else return;event.preventDefault();select(tabs[n].dataset.example,true);});
  });
  fetch(new URL('../../assets/home-examples.json',script.src)).then(response=>{if(!response.ok)throw new Error('Examples unavailable');return response.json();}).then(data=>{examples=data;tabs.forEach(tab=>tab.disabled=false);select('lookup');}).catch(()=>{next.textContent='The interactive comparison could not load. Use the playground link to run the example.';});
})();
