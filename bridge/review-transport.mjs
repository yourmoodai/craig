import {connect} from 'node:net';

export function exchange_at(endpoint,request,timeout) {
  return new Promise((done,reject)=>{
    const socket=connect(endpoint);
    let size=0; const chunks=[];
    const deadline=setTimeout(()=>socket.destroy(Error('review_timeout')),timeout);
    socket.on('close',()=>{clearTimeout(deadline);reject(Error('review_closed'));});
    socket.on('error',()=>reject(Error('review_unavailable')));
    socket.on('connect',()=>socket.write(JSON.stringify(request)+'\n'));
    socket.on('data',chunk=>{size+=chunk.length;if(size>16*1024*1024){socket.destroy(Error('review_response_too_large'));return;}chunks.push(chunk);}); // 16 MiB
    socket.on('end',()=>{try{const response=JSON.parse(Buffer.concat(chunks).toString('utf8'));if(response.version!==1 || response.status!=='ok') throw Error(response.code||'invalid_review_response');done(response.value);}catch{reject(Error('review_request_failed'));}});
  });
}
