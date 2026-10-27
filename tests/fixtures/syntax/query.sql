-- 최근 주문 조회
SELECT o.id, o.total, c.name AS customer
FROM orders o
INNER JOIN customers c ON c.id = o.customer_id
WHERE o.created_at >= '2026-10-01'
  AND o.status <> 'cancelled'
ORDER BY o.total DESC
LIMIT 20;

/* 집계 */
select count(*) from orders where total > 1000;
