"""모듈 설명 — 여러 줄
문서 문자열"""
import os


@staticmethod
def greet(name: str, times: int = 3) -> None:
    # 인사를 여러 번
    for i in range(times):
        print(f"안녕, {name}! ({i + 1})")
    return None


if __name__ == "__main__":
    greet(os.environ.get("USER", 'world'))
