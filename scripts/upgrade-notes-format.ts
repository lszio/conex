import { getCanonicalStore } from "/Users/lszio/Projects/conex/src/store/canonical-store.js";
import { AppleRemindersAdapter } from "/Users/lszio/Projects/conex/src/adapters/apple/reminders.js";
import { canonicalToReminder } from "/Users/lszio/Projects/conex/src/mappers/reminder-mapper.js";

async function main() {
  const store = getCanonicalStore();
  const reminders = new AppleRemindersAdapter();
  const all = store.list({limit: 200});
  
  let upgraded = 0;
  let skipped = 0;
  let errors = 0;
  
  for (const task of all) {
    if (!task.apple_id) { skipped++; continue; }
    
    try {
      const existing = await reminders.getReminder(task.apple_id);
      if (!existing) { skipped++; continue; }
      
      // Check if notes have the old format
      const hasNewFormat = existing.notes?.includes("─── CONEX ───");
      if (hasNewFormat) { skipped++; continue; }
      
      // Rebuild notes with new format
      const newReminder = canonicalToReminder(task);
      await reminders.updateReminder(task.apple_id, newReminder);
      console.log(`✅ ${task.name} — notes upgraded`);
      upgraded++;
    } catch (e: any) {
      console.error(`❌ ${task.name}: ${e.message}`);
      errors++;
    }
  }
  
  console.log(`\nDone: ${upgraded} upgraded, ${skipped} skipped, ${errors} errors`);
  
  // Verify one sample
  if (upgraded > 0) {
    const sample = all.find(t => t.apple_id);
    if (sample) {
      const v = await reminders.getReminder(sample.apple_id!);
      console.log(`\nSample (${sample.name}) notes:\n${v?.notes || "(empty)"}`);
    }
  }
}

main().catch(console.error);
