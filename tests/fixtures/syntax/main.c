#include <stdio.h>
#define MAX 10

/* 합계를 구한다 */
static int sum(const int *values, int count) {
    int total = 0;
    for (int i = 0; i < count; i++) {
        total += values[i];   // 누적
    }
    return total;
}

int main(void) {
    int values[MAX] = {1, 2, 3};
    printf("합계: %d\n", sum(values, 3));
    return 0;
}
