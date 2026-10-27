// 간단한 서버
import http from "node:http";

const PORT = Number(process.env.PORT ?? 3000);

async function handle(req, res) {
  /* 상태 확인 */
  if (req.url === "/health") {
    res.end(JSON.stringify({ ok: true }));
    return;
  }
  res.writeHead(404);
  res.end(`없는 경로: ${req.url}`);
}

http.createServer(handle).listen(PORT, () => console.log('listening', PORT));
