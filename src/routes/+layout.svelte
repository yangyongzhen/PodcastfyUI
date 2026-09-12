<script lang="ts">
  import "../app.css";
  import Toaster from "../lib/ui/Toaster.svelte";
  import ConfirmDialog from "../lib/ui/ConfirmDialog.svelte";

  let { children } = $props();
</script>

{@render children()}
<Toaster />
<ConfirmDialog />
