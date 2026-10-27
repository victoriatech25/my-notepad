using System;

namespace Demo
{
    // 진입점
    public static class Program
    {
        public static async Task Main(string[] args)
        {
            var name = args.Length > 0 ? args[0] : "세계";
            Console.WriteLine($"안녕, {name}!");
            await Task.Delay(100);
        }
    }
}
