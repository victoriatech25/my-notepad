package main

import (
	"fmt"
	"os"
)

// 진입점
func main() {
	name := "세계"
	if len(os.Args) > 1 {
		name = os.Args[1]
	}
	raw := `여러 줄
원시 문자열`
	fmt.Printf("안녕, %s! %d\n", name, 42)
	_ = raw
}
