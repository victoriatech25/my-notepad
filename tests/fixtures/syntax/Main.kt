package demo

// 데이터 클래스
data class User(val id: Int, val name: String)

fun main() {
    val users = listOf(User(1, "가"), User(2, "나"))
    for (u in users) {
        when {
            u.id > 1 -> println("다음: ${u.name}")
            else -> println("처음")
        }
    }
}
