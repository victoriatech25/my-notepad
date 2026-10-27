package demo;

import java.util.List;

/** 진입점 */
public class Main {
    private static final int LIMIT = 3;

    public static void main(String[] args) {
        List<String> names = List.of("가", "나", "다");
        for (int i = 0; i < LIMIT; i++) {
            System.out.println(names.get(i)); // 출력
        }
    }
}
