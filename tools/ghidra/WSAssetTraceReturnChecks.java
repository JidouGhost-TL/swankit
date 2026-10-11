// SPDX-License-Identifier: MIT OR Apache-2.0
// @category WonderSwan.Assets
// Synthetic RAM programs only; no cartridge bytes are used as fixtures.
import jidoughost.wonderswan.WSMachine;
import java.nio.file.*;
import java.io.*;
import java.util.*;
public class WSAssetTraceReturnChecks extends WSAssetTrace {
 @Override public void run() throws Exception {
  Path out=Paths.get(getScriptArgs()[0]);Files.createDirectories(out);List<String> failures=new ArrayList<>();
  for(int variant=0;variant<3;variant++) {
   cells.clear();immediates.clear();stack.clear();pendingCall=false;returned=false;nextId=0;dmaSeen=0;
   machine=new WSMachine(currentProgram,true);StringWriter captured=new StringWriter();calls=new PrintWriter(captured);accesses=new PrintWriter(new StringWriter());installCallbacks();
   ByteArrayOutputStream code=new ByteArrayOutputStream();
   code.write(new byte[]{0x68,0x34,0x12}); // PUSH synthetic argument.
   if(variant==2)code.write(new byte[]{(byte)0x9a,0x0b,0x10,0,0}); // CALLF 0000:100b.
   else code.write(new byte[]{(byte)0xe8,0x03,0x00}); // CALL 1009.
   code.write(new byte[]{(byte)0x90,(byte)0xeb,(byte)0xfe}); // NOP after return; idle.
   for(int i=0;i<4;i++)code.write(new byte[]{(byte)0xa3,(byte)(i*2),0x20}); // MOV [2000+2*i],AX.
   code.write(variant==0?new byte[]{(byte)0xc3}:new byte[]{(byte)(variant==2?0xca:0xc2),0x02,0x00}); // RET / RET 2 / RETF 2.
   machine.write(0x1000,code.toByteArray());
   for(String reg:List.of("CS","DS","ES","SS"))machine.setReg(machine.lang.getRegister(reg),0);
   machine.setReg(machine.lang.getRegister("SP"),0x8000);machine.setReg(machine.lang.getRegister("AX"),0x0102);
   var ax=machine.lang.getRegister("AX");put(ax.getAddress().getAddressSpace(),ax.getAddress().getOffset(),2,Set.of(123L));
   machine.thread.overrideCounter(machine.ram.getAddress(0x1000));
   machine.run(1,159,f->{}); // Includes the extension's CS-context synchronization.

   calls.flush();String result=captured.toString();Files.writeString(out.resolve("variant-"+variant+".jsonl"),result);
   if(!result.contains("\"complete_return\":true") || !stack.isEmpty())failures.add("variant "+variant+": call not paired after return (cdecl / near cleanup / far cleanup)");
  }
  Files.writeString(out.resolve("checks.json"),"{\"checks\":3,\"failures\":"+listStrings(failures)+"}");
  println("RETURN TESTS: "+failures);if(!failures.isEmpty())throw new IllegalStateException("return tests failed");
 }
}
