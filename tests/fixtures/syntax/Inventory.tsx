import { useState } from "react";

// 재고 목록 화면
interface Item {
  id: number;
  name: string;
}

export default function Inventory({ items }: { items: Item[] }) {
  const [query, setQuery] = useState<string>("");
  const shown = items.filter((i) => i.name.includes(query));
  /* 비어 있으면 안내 */
  if (shown.length === 0) {
    return <p className="empty">결과 없음</p>;
  }
  return (
    <ul>
      {shown.map((i) => (
        <li key={i.id}>{`${i.id}: ${i.name}`}</li>
      ))}
    </ul>
  );
}
