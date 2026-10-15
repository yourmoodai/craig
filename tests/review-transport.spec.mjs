import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:net';
import {exchange_at} from '../bridge/review-transport.mjs';

async function with_server(handle,check) {
  const sockets=new Set();
  const server=createServer(socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.on('error',()=>{});handle(socket);});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try {await check({host:'127.0.0.1',port:server.address().port});}
  finally {for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));}
}
test('review transport rejects missing and truncated replies',async()=>{
  for(const body of ['', '{"version":1'])await with_server(socket=>socket.once('data',()=>socket.end(body)),async endpoint=>{
    await assert.rejects(exchange_at(endpoint,{command:'status'},500),/review_request_failed|review_closed/);
  });
});
test('review deadline bounds silent and trickling peers',async()=>{
  for(const trickle of [false,true])await with_server(socket=>{
    socket.on('data',()=>{});
    if(trickle){const timer=setInterval(()=>socket.write(' '),5);socket.on('close',()=>clearInterval(timer));}
  },async endpoint=>{
    const start=Date.now();
    await assert.rejects(exchange_at(endpoint,{command:'status'},50));
    assert.ok(Date.now()-start<1000);
  });
});
test('review transport accepts one complete protocol response',async()=>{
  await with_server(socket=>socket.once('data',()=>socket.end(JSON.stringify({version:1,status:'ok',value:{ready:true}}))),async endpoint=>{
    assert.deepEqual(await exchange_at(endpoint,{command:'status'},500),{ready:true});
  });
});
