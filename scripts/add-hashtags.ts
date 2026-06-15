import { getCanonicalStore } from "/Users/lszio/Projects/conex/src/store/canonical-store.js";
import { AppleRemindersAdapter } from "/Users/lszio/Projects/conex/src/adapters/apple/reminders.js";
import { canonicalToReminder } from "/Users/lszio/Projects/conex/src/mappers/reminder-mapper.js";

async function main() {
  const store = getCanonicalStore();
  const reminders = new AppleRemindersAdapter();
  const all = store.list({limit: 200});
  
  let added = 0, skipped = 0, errors = 0;
  
  for (const task of all) {
    if (!task.apple_id) { skipped++; continue; }
    
    try {
      const existing = await reminders.getReminder(task.apple_id);
      if (!existing) { skipped++; continue; }
      
      // Check if notes already have #conex tag
      const hasConexTag = existing.notes?.includes("#conex");
      if (hasConexTag) { skipped++; continue; }
      
      // Rebuild notes with new buildNotes (includes #conex + tags)
      const newReminder = canonicalToReminder(task);
      await reminders.updateReminder(task.apple_id, newReminder);
      console.log(`✅ ${task.name}`);
      added++;
    } catch (e: any) {
      console.error(`❌ ${task.name}: ${e.message}`);
      errors++;
    }
  }
  
  console.log(`\nDone: ${added} added hashtags, ${skipped} skipped, ${errors} errors`);
}

main().catch(console.error);
